//! Where a runtime runs.
//!
//! Colab picks the place, and neither the CLI nor the API reports it, but the host of a
//! runtime's proxy URL names its Google Cloud region (`8080-…-b.us-west1.prod.r.colab.dev`).
//! Colab does not document that host, so a host without a label shaped like a region leaves
//! the region unknown rather than wrong.

/// The Google Cloud region a runtime proxy host names, such as `us-west1`.
pub fn region_of_host(host: &str) -> Option<String> {
    host.split('.')
        .find(|label| is_region(label))
        .map(str::to_owned)
}

/// An area, a dash, a direction and a number: `us-west1`, `europe-west10`,
/// `northamerica-northeast2`. A zone (`us-west1-b`) or a port-and-endpoint label is not one.
fn is_region(label: &str) -> bool {
    let Some((area, rest)) = label.split_once('-') else {
        return false;
    };
    let number = rest.trim_start_matches(|c: char| c.is_ascii_lowercase());
    !area.is_empty()
        && area.bytes().all(|b| b.is_ascii_lowercase())
        && number.len() < rest.len()
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
}

/// A country or territory with a Google Cloud region, named as Google's region list names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Country {
    Australia,
    Belgium,
    Brazil,
    Canada,
    Chile,
    Finland,
    France,
    Germany,
    HongKong,
    India,
    Indonesia,
    Israel,
    Italy,
    Japan,
    Mexico,
    Netherlands,
    Poland,
    Qatar,
    SaudiArabia,
    Singapore,
    SouthAfrica,
    SouthKorea,
    Spain,
    Sweden,
    Switzerland,
    Taiwan,
    Thailand,
    UnitedKingdom,
    UnitedStates,
}

/// Where a region is, from Google's list of Compute Engine regions (43 regions, read on
/// 2026-11-27). A region opened since then is `None`, and its name is shown on its own.
pub fn country_of_region(region: &str) -> Option<Country> {
    use Country::*;
    Some(match region {
        "africa-south1" => SouthAfrica,
        "asia-east1" => Taiwan,
        "asia-east2" => HongKong,
        "asia-northeast1" | "asia-northeast2" => Japan,
        "asia-northeast3" => SouthKorea,
        "asia-south1" | "asia-south2" => India,
        "asia-southeast1" => Singapore,
        "asia-southeast2" => Indonesia,
        "asia-southeast3" => Thailand,
        "australia-southeast1" | "australia-southeast2" => Australia,
        "europe-central2" => Poland,
        "europe-north1" => Finland,
        "europe-north2" => Sweden,
        "europe-southwest1" => Spain,
        "europe-west1" => Belgium,
        "europe-west2" => UnitedKingdom,
        "europe-west3" | "europe-west10" => Germany,
        "europe-west4" => Netherlands,
        "europe-west6" => Switzerland,
        "europe-west8" | "europe-west12" => Italy,
        "europe-west9" => France,
        "me-central1" => Qatar,
        "me-central2" => SaudiArabia,
        "me-west1" => Israel,
        "northamerica-northeast1" | "northamerica-northeast2" => Canada,
        "northamerica-south1" => Mexico,
        "southamerica-east1" => Brazil,
        "southamerica-west1" => Chile,
        "us-central1" | "us-east1" | "us-east4" | "us-east5" | "us-south1" | "us-west1"
        | "us-west2" | "us-west3" | "us-west4" => UnitedStates,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_region_is_the_label_shaped_like_one() {
        // The host of a CPU runtime on 2026-11-27 (docs/COLAB_NOTES.md).
        let seen = "8080-m-s-usw1b11-1xg80oa3u7s0b-b.us-west1.prod.r.colab.dev";
        assert_eq!(region_of_host(seen).as_deref(), Some("us-west1"));
        for region in ["europe-west10", "northamerica-northeast2", "me-central2"] {
            let host = format!("8080-m-s-x-b.{region}.prod.r.colab.dev");
            assert_eq!(region_of_host(&host).as_deref(), Some(region));
        }
    }

    #[test]
    fn a_host_without_such_a_label_has_no_region() {
        for host in [
            "abc-foo.colab.googleusercontent.com",
            "8080-m-s-x-b.us-west1-b.prod.r.colab.dev",
            "proxy.invalid",
            "localhost",
            "",
        ] {
            assert_eq!(region_of_host(host), None, "{host}");
        }
    }

    #[test]
    fn a_region_is_where_google_lists_it() {
        assert_eq!(country_of_region("us-west1"), Some(Country::UnitedStates));
        assert_eq!(
            country_of_region("europe-west2"),
            Some(Country::UnitedKingdom)
        );
        assert_eq!(country_of_region("asia-east2"), Some(Country::HongKong));
        assert_eq!(country_of_region("europe-west99"), None);
    }
}
