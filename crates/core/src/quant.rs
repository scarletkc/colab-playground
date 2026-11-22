//! GGUF quantisation grouping and VRAM fitting.
//!
//! The same algorithm runs on the VM inside the bundled `llama-cpp` worker
//! (`plugins/llama-cpp/worker.py`), which knows the real GPU and reads the
//! model's GGUF header; here it powers UI previews ("80 GB → Q8_0"). Both
//! implementations are checked against `tests/data/quant_fit_vectors.json`.
//!
//! Estimate = weights + mmproj + KV cache + overhead, where
//! KV = bytes/token (f16) × KV-type factor × context, and
//! overhead = 1024 MiB reserve + 512 MiB + 5 % of the weights (compute buffers).
//! The budget is 97 % of the GPU memory.
//!
//! `auto` walks, in order: normal quants (Q4 and above) before low-bit ones;
//! the requested context (or the model's maximum, halving down to 8192 when
//! `ctx = 0`); quants from largest to smallest (full-precision F16/BF16/F32
//! only if nothing else exists); KV `f16` then `q8_0` (when the KV type is
//! `auto`). The first combination that fits wins.

use serde::{Deserialize, Serialize};

use crate::{
    error::{Error, ErrorKind, Result},
    sessions::Accelerator,
};

/// Id of the bundled `llama-cpp` plugin's input for the context length (`plugins/llama-cpp`).
pub const INPUT_CTX: &str = "ctx";
/// Id of the bundled `llama-cpp` plugin's input for the quantisation.
pub const INPUT_QUANT: &str = "quant";
/// The quantisation that leaves the choice to the worker on the VM.
pub const QUANT_AUTO: &str = "auto";

/// GPU memory sizes (GiB) Colab hands out for an accelerator: an A100 is 40 or 80 GB, and
/// which one cannot be requested. Empty for hardware without a GPU.
pub fn vram_choices_gib(accelerator: Accelerator) -> &'static [u32] {
    match accelerator {
        Accelerator::T4 => &[15],
        Accelerator::L4 => &[22],
        Accelerator::A100 => &[40, 80],
        Accelerator::H100 => &[80],
        Accelerator::G4 => &[96],
        Accelerator::None | Accelerator::TpuV5e1 | Accelerator::TpuV6e1 => &[],
    }
}

/// MiB reserved for the CUDA context and driver.
pub const RESERVE_MIB: f64 = 1024.0;
/// Fixed part of the compute-buffer estimate.
pub const COMPUTE_MIB: f64 = 512.0;
/// Compute buffers as a fraction of the weights.
pub const COMPUTE_FRACTION: f64 = 0.05;
/// Fraction of GPU memory that may be planned.
pub const BUDGET_FRACTION: f64 = 0.97;
/// KV bytes per token when the GGUF header could not be read.
pub const FALLBACK_KV_BYTES_PER_TOKEN: u64 = 128 * 1024;
/// Context assumed when the model's maximum is unknown.
pub const FALLBACK_CTX: u32 = 32_768;
/// Smallest context the `ctx = 0` ladder goes down to.
pub const MIN_AUTO_CTX: u32 = 8_192;
/// Largest context the `ctx = 0` ladder starts from, whatever the model allows; a context the
/// user sets is used as is.
pub const AUTO_CTX_CAP: u32 = 131_072;
/// Compute buffers per 1,024 tokens of context, on top of the fixed part: llama.cpp's buffers
/// grow with the context, which a fixed estimate missed for very long ones. A margin, not a
/// measurement.
pub const COMPUTE_MIB_PER_1K_CTX: f64 = 8.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GgufFile {
    pub path: String,
    /// Size in bytes.
    pub size: u64,
}

/// All files of one quantisation (several when sharded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GgufVariant {
    /// Label such as `Q8_0`, `Q4_K_M`, `UD-Q4_K_XL`, `BF16`.
    pub quant: String,
    /// File paths (sorted; first shard first).
    pub files: Vec<String>,
    /// Total size in bytes.
    pub size: u64,
}

impl GgufVariant {
    pub fn is_full_precision(&self) -> bool {
        matches!(self.quant.as_str(), "F16" | "BF16" | "F32")
    }

    /// Below 4 bits (Q3, Q2, IQ3, IQ2, IQ1, TQ).
    pub fn is_low_bit(&self) -> bool {
        let q = self.quant.trim_start_matches("UD-");
        ["Q3", "Q2", "Q1", "IQ3", "IQ2", "IQ1", "TQ"]
            .iter()
            .any(|p| q.starts_with(p))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GgufListing {
    /// Quantisations, largest first.
    pub variants: Vec<GgufVariant>,
    /// Multimodal projector files.
    pub mmproj: Vec<GgufFile>,
}

impl GgufListing {
    /// Finds a variant by label (case-insensitive).
    pub fn variant(&self, quant: &str) -> Option<&GgufVariant> {
        self.variants
            .iter()
            .find(|v| v.quant.eq_ignore_ascii_case(quant))
    }

    /// The preferred projector (f16 first, then bf16, then any).
    pub fn preferred_mmproj(&self) -> Option<&GgufFile> {
        let rank = |p: &str| {
            let l = p.to_ascii_lowercase();
            if l.contains("-f16") || l.contains("_f16") {
                0
            } else if l.contains("bf16") {
                1
            } else {
                2
            }
        };
        self.mmproj
            .iter()
            .min_by_key(|f| (rank(&f.path), f.path.clone()))
    }
}

fn is_quant_token(t: &str) -> bool {
    let u = t.to_ascii_uppercase();
    if !u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    if matches!(u.as_str(), "F16" | "F32" | "BF16") || u.starts_with("MXFP4") {
        return true;
    }
    for prefix in ["IQ", "TQ", "Q"] {
        if let Some(rest) = u.strip_prefix(prefix) {
            return rest.chars().next().is_some_and(|c| c.is_ascii_digit());
        }
    }
    false
}

fn strip_shard_suffix(stem: &str) -> &str {
    // "-00001-of-00003". Matched on bytes: the name can hold multi-byte
    // characters, so a byte offset into it is not always a `str` boundary.
    const SUFFIX_LEN: usize = 15;
    let b = stem.as_bytes();
    let Some(start) = b.len().checked_sub(SUFFIX_LEN) else {
        return stem;
    };
    let tail = &b[start..];
    let digits = |r: std::ops::Range<usize>| tail[r].iter().all(u8::is_ascii_digit);
    if tail[0] == b'-' && digits(1..6) && &tail[6..10] == b"-of-" && digits(10..15) {
        // The suffix is ASCII, so it starts on a boundary.
        return &stem[..start];
    }
    stem
}

pub fn quant_label(path: &str) -> Option<String> {
    let base = path.rsplit('/').next().unwrap_or(path);
    if !base.to_ascii_lowercase().ends_with(".gguf") {
        return None;
    }
    let stem = strip_shard_suffix(&base[..base.len() - 5]);
    let tokens: Vec<&str> = stem.split(['-', '.']).collect();
    for i in (0..tokens.len()).rev() {
        if is_quant_token(tokens[i]) {
            let label = tokens[i].to_ascii_uppercase();
            if i > 0 && tokens[i - 1].eq_ignore_ascii_case("ud") {
                return Some(format!("UD-{label}"));
            }
            return Some(label);
        }
    }
    None
}

pub fn is_mmproj(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
        .contains("mmproj")
}

/// Groups GGUF files into variants (largest first) and projectors.
pub fn group(files: &[GgufFile]) -> GgufListing {
    let mut listing = GgufListing::default();
    let mut by_label: std::collections::BTreeMap<String, GgufVariant> = Default::default();
    for f in files
        .iter()
        .filter(|f| f.path.to_ascii_lowercase().ends_with(".gguf"))
    {
        if is_mmproj(&f.path) {
            listing.mmproj.push(f.clone());
            continue;
        }
        let Some(label) = quant_label(&f.path) else {
            continue;
        };
        let v = by_label.entry(label.clone()).or_insert(GgufVariant {
            quant: label,
            files: vec![],
            size: 0,
        });
        v.files.push(f.path.clone());
        v.size += f.size;
    }
    for v in by_label.values_mut() {
        v.files.sort();
    }
    listing.variants = by_label.into_values().collect();
    listing
        .variants
        .sort_by(|a, b| b.size.cmp(&a.size).then(a.quant.cmp(&b.quant)));
    listing
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KvType {
    F16,
    /// 8-bit blocks (34 bytes per 32 values).
    Q8_0,
    /// 4-bit blocks (18 bytes per 32 values).
    Q4_0,
}

impl KvType {
    /// Size relative to f16.
    pub fn factor(self) -> f64 {
        match self {
            KvType::F16 => 1.0,
            KvType::Q8_0 => 34.0 / 64.0,
            KvType::Q4_0 => 18.0 / 64.0,
        }
    }

    /// llama.cpp `--cache-type-k/v` value.
    pub fn as_str(self) -> &'static str {
        match self {
            KvType::F16 => "f16",
            KvType::Q8_0 => "q8_0",
            KvType::Q4_0 => "q4_0",
        }
    }

    pub fn parse(s: &str) -> Option<KvType> {
        match s.to_ascii_lowercase().as_str() {
            "f16" => Some(KvType::F16),
            "q8_0" => Some(KvType::Q8_0),
            "q4_0" => Some(KvType::Q4_0),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitInput {
    /// Total GPU memory in MiB.
    pub vram_mib: u32,
    /// Requested context; 0 = model maximum (with the halving ladder).
    pub ctx: u32,
    /// The model's maximum context (`<arch>.context_length`), if known.
    pub model_ctx: Option<u32>,
    /// KV bytes per token at f16, if known from the GGUF header.
    pub kv_bytes_per_token: Option<u64>,
    /// Projector size in bytes (0 if none).
    pub mmproj_bytes: u64,
    /// Fixed KV type, or `None` for auto (f16, then q8_0).
    pub kv: Option<KvType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitChoice {
    pub quant: String,
    pub kv: KvType,
    /// Context to pass to llama.cpp.
    pub ctx: u32,
    /// Estimated footprint in MiB.
    pub estimate_mib: u32,
    /// Planning budget in MiB.
    pub budget_mib: u32,
    /// The context was reduced from the model maximum.
    pub ctx_reduced: bool,
    /// The KV cache was quantised to fit.
    pub kv_reduced: bool,
}

/// Estimated footprint in MiB.
pub fn estimate_mib(
    weights: u64,
    mmproj: u64,
    kv_bytes_per_token: u64,
    ctx: u32,
    kv: KvType,
) -> f64 {
    let mib = |b: f64| b / (1024.0 * 1024.0);
    let w = mib(weights as f64);
    let kv_mib = mib(kv_bytes_per_token as f64 * kv.factor() * f64::from(ctx));
    w + mib(mmproj as f64)
        + kv_mib
        + RESERVE_MIB
        + COMPUTE_MIB
        + COMPUTE_FRACTION * w
        + COMPUTE_MIB_PER_1K_CTX * f64::from(ctx) / 1024.0
}

fn budget(vram_mib: u32) -> f64 {
    f64::from(vram_mib) * BUDGET_FRACTION
}

fn ctx_ladder(input: &FitInput) -> (Vec<u32>, u32) {
    if input.ctx > 0 {
        return (vec![input.ctx], input.ctx);
    }
    let max = input
        .model_ctx
        .filter(|c| *c > 0)
        .unwrap_or(FALLBACK_CTX)
        .min(AUTO_CTX_CAP);
    let mut v = vec![max];
    let mut c = max;
    while c / 2 >= MIN_AUTO_CTX {
        c /= 2;
        v.push(c);
    }
    (v, max)
}

fn kv_options(input: &FitInput) -> Vec<KvType> {
    match input.kv {
        Some(k) => vec![k],
        None => vec![KvType::F16, KvType::Q8_0],
    }
}

fn try_fit(v: &GgufVariant, input: &FitInput, ctx: u32, max_ctx: u32) -> Option<FitChoice> {
    let kvb = input
        .kv_bytes_per_token
        .unwrap_or(FALLBACK_KV_BYTES_PER_TOKEN);
    let b = budget(input.vram_mib);
    for kv in kv_options(input) {
        let est = estimate_mib(v.size, input.mmproj_bytes, kvb, ctx, kv);
        if est <= b {
            return Some(FitChoice {
                quant: v.quant.clone(),
                kv,
                ctx,
                estimate_mib: est.ceil() as u32,
                budget_mib: b.floor() as u32,
                ctx_reduced: ctx < max_ctx,
                kv_reduced: input.kv.is_none() && kv != KvType::F16,
            });
        }
    }
    None
}

/// Picks the best quantisation that fits (`quant = auto`).
pub fn fit_auto(listing: &GgufListing, input: &FitInput) -> Result<FitChoice> {
    let quantised: Vec<&GgufVariant> = listing
        .variants
        .iter()
        .filter(|v| !v.is_full_precision())
        .collect();
    let pool: Vec<&GgufVariant> = if quantised.is_empty() {
        listing.variants.iter().collect()
    } else {
        quantised
    };
    let normal: Vec<&GgufVariant> = pool.iter().copied().filter(|v| !v.is_low_bit()).collect();
    let (ladder, max_ctx) = ctx_ladder(input);
    for tier in [normal, pool.clone()] {
        for &ctx in &ladder {
            for v in &tier {
                if let Some(c) = try_fit(v, input, ctx, max_ctx) {
                    return Ok(c);
                }
            }
        }
    }
    Err(Error::new(ErrorKind::InvalidInput)
        .with("field", INPUT_QUANT)
        .with("vram_mib", input.vram_mib)
        .detail("no quantisation fits this GPU"))
}

/// Checks a fixed quantisation; on failure the error lists those that would fit
/// (param `fitting`, comma-separated).
pub fn fit_fixed(listing: &GgufListing, quant: &str, input: &FitInput) -> Result<FitChoice> {
    let Some(v) = listing.variant(quant) else {
        let available: Vec<&str> = listing.variants.iter().map(|v| v.quant.as_str()).collect();
        return Err(
            Error::invalid(INPUT_QUANT, format!("{quant} is not in the repository"))
                .with("value", quant)
                .with("available", available.join(",")),
        );
    };
    let (ladder, max_ctx) = ctx_ladder(input);
    for &ctx in &ladder {
        if let Some(c) = try_fit(v, input, ctx, max_ctx) {
            return Ok(c);
        }
    }
    let fitting: Vec<&str> = listing
        .variants
        .iter()
        .filter(|w| {
            ladder
                .iter()
                .any(|&ctx| try_fit(w, input, ctx, max_ctx).is_some())
        })
        .map(|w| w.quant.as_str())
        .collect();
    Err(Error::invalid(
        INPUT_QUANT,
        format!("{quant} does not fit in {} MiB", input.vram_mib),
    )
    .with("value", quant)
    .with("fitting", fitting.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn f(path: &str, size: u64) -> GgufFile {
        GgufFile {
            path: path.into(),
            size,
        }
    }

    #[test]
    fn labels() {
        assert_eq!(
            quant_label("Qwen3-0.6B-Q4_K_M.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(quant_label("model.Q8_0.gguf").as_deref(), Some("Q8_0"));
        assert_eq!(
            quant_label("x/Model-UD-Q4_K_XL.gguf").as_deref(),
            Some("UD-Q4_K_XL")
        );
        assert_eq!(
            quant_label("Q8_0/Big-Q8_0-00002-of-00003.gguf").as_deref(),
            Some("Q8_0")
        );
        assert_eq!(quant_label("m-bf16.gguf").as_deref(), Some("BF16"));
        assert_eq!(quant_label("m-IQ4_XS.gguf").as_deref(), Some("IQ4_XS"));
        assert_eq!(
            quant_label("m-MXFP4_MOE.gguf").as_deref(),
            Some("MXFP4_MOE")
        );
        assert_eq!(quant_label("README.md"), None);
        assert_eq!(quant_label("model.gguf"), None);
    }

    #[test]
    fn labels_of_non_ascii_names() {
        // Multi-byte characters at the byte offsets where a shard suffix
        // would start, and inside it.
        assert_eq!(quant_label("模型模型模型模型模a.gguf"), None);
        assert_eq!(quant_label("m-00001模模模.gguf"), None);
        assert_eq!(
            quant_label("模型模型模型模型-Q4_K_M.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(
            quant_label("模型-Q8_0-00001-of-00003.gguf").as_deref(),
            Some("Q8_0")
        );
        assert_eq!(strip_shard_suffix("模型-00001-of-00003"), "模型");

        let l = group(&[
            f("模型模型模型模型-Q4_K_M.gguf", 2 * GIB),
            f("模型-Q8_0-00001-of-00002.gguf", 3 * GIB),
            f("模型-Q8_0-00002-of-00002.gguf", GIB),
            f("模型模型模型模型模a.gguf", GIB),
        ]);
        let quants: Vec<&str> = l.variants.iter().map(|v| v.quant.as_str()).collect();
        assert_eq!(quants, ["Q8_0", "Q4_K_M"]);
        assert_eq!(l.variants[0].files.len(), 2);
    }

    #[test]
    fn grouping_shards_and_mmproj() {
        let l = group(&[
            f("Q8_0/M-Q8_0-00001-of-00002.gguf", 20 * GIB),
            f("Q8_0/M-Q8_0-00002-of-00002.gguf", 7 * GIB),
            f("M-Q4_K_M.gguf", 15 * GIB),
            f("mmproj-M-f16.gguf", GIB),
            f("mmproj-M-bf16.gguf", GIB),
            f("README.md", 10),
        ]);
        assert_eq!(l.variants[0].quant, "Q8_0");
        assert_eq!(l.variants[0].size, 27 * GIB);
        assert_eq!(l.variants[0].files.len(), 2);
        assert!(l.variants[0].files[0].contains("00001"));
        assert_eq!(l.preferred_mmproj().unwrap().path, "mmproj-M-f16.gguf");
    }

    #[derive(Deserialize)]
    struct Vector {
        name: String,
        files: Vec<(String, u64)>,
        input: FitInput,
        quant: String,
        expect: Option<Expect>,
        expect_error_fitting: Option<Vec<String>>,
    }

    #[derive(Deserialize)]
    struct Expect {
        quant: String,
        kv: KvType,
        ctx: u32,
    }

    /// Shared with `worker.py --fit-self-test`.
    #[test]
    fn shared_fit_vectors() {
        let vectors: Vec<Vector> =
            serde_json::from_str(include_str!("../tests/data/quant_fit_vectors.json")).unwrap();
        assert!(vectors.len() >= 5);
        for v in vectors {
            let files: Vec<GgufFile> = v.files.iter().map(|(p, s)| f(p, *s)).collect();
            let listing = group(&files);
            let r = if v.quant == QUANT_AUTO {
                fit_auto(&listing, &v.input)
            } else {
                fit_fixed(&listing, &v.quant, &v.input)
            };
            match (r, &v.expect, &v.expect_error_fitting) {
                (Ok(c), Some(e), _) => {
                    assert_eq!(
                        (c.quant.as_str(), c.kv, c.ctx),
                        (e.quant.as_str(), e.kv, e.ctx),
                        "{}",
                        v.name
                    );
                }
                (Err(err), None, Some(fitting)) => {
                    assert_eq!(
                        err.param("fitting").unwrap_or(""),
                        fitting.join(","),
                        "{}",
                        v.name
                    );
                }
                (r, _, _) => panic!("{}: unexpected {r:?}", v.name),
            }
        }
    }
}
