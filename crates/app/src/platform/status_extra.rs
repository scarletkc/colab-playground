//! The macOS menu bar extra: live session count, burn rate, and GPU names.

use anyhow::Context as _;
use block2::RcBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool},
    sel,
};
use objc2_app_kit::{
    NSCellImagePosition, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSImage, NSImageScaling, NSStatusBar, NSStatusItem, NSStringDrawing,
    NSVariableStatusItemLength,
};
use objc2_foundation::{NSDictionary, NSObject, NSPoint, NSRect, NSSize, NSString};

use super::StatusLines;

/// Horizontal inset so the extra does not touch its neighbours.
const TEXT_INSET_X: f64 = 4.0;

/// Extra width so the measured string is not clipped when drawn.
const WIDTH_SLACK: f64 = 2.0;

type OnClick = Box<dyn Fn()>;

define_class!(
    // SAFETY: NSObject has no subclassing rules; the class adds no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ColabPlaygroundStatusTarget"]
    #[ivars = OnClick]
    struct Target;

    impl Target {
        #[unsafe(method(statusClicked:))]
        fn clicked(&self, _: Option<&AnyObject>) {
            (self.ivars())();
        }
    }
);

impl Target {
    fn new(on_click: OnClick, mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(on_click);
        // SAFETY: NSObject's init takes no arguments and returns the object.
        unsafe { msg_send![super(this), init] }
    }
}

fn main_thread() -> MainThreadMarker {
    MainThreadMarker::new().unwrap_or_else(|| panic!("the menu bar extra runs on the main thread"))
}

/// A text extra in the menu bar; dropping it removes it.
pub(super) struct Item {
    item: Retained<NSStatusItem>,
    _target: Retained<Target>,
}

impl Item {
    pub(super) fn new(on_click: OnClick) -> anyhow::Result<Self> {
        let mtm = main_thread();
        let target = Target::new(on_click, mtm);
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let button = item.button(mtm).context("status item has no button")?;
        // SAFETY: the target answers `statusClicked:` with the sender.
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(statusClicked:)));
        }
        log::info!("status extra: shown");
        Ok(Self {
            item,
            _target: target,
        })
    }

    pub(super) fn apply(&self, lines: &StatusLines) {
        let mtm = main_thread();
        let Some(button) = self.item.button(mtm) else {
            return;
        };
        let image = paint(lines);
        // Title plus image would sit side by side; the extra is the image alone.
        button.setTitle(&NSString::from_str(""));
        button.setImage(Some(&image));
        button.setImagePosition(NSCellImagePosition::ImageOnly);
        button.setImageScaling(NSImageScaling::ScaleNone);
        self.item.setLength(image.size().width);
        button.setToolTip(Some(&NSString::from_str(&lines.tooltip)));
    }
}

impl Drop for Item {
    fn drop(&mut self) {
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
        log::info!("status extra: removed");
    }
}

/// A template image follows the menu bar's light/dark tint; an attributed title does not.
fn paint(lines: &StatusLines) -> Retained<NSImage> {
    let thickness = NSStatusBar::systemStatusBar().thickness();
    let font = NSFont::menuBarFontOfSize(0.0);
    let color = NSColor::blackColor();
    // SAFETY: the keys are the font and colour attributes; the values match.
    let attrs = unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[font.as_ref() as &AnyObject, color.as_ref() as &AnyObject],
        )
    };
    let text = NSString::from_str(&lines.text);
    // SAFETY: `attrs` is a valid string-drawing dictionary.
    let text_size = unsafe { text.sizeWithAttributes(Some(&attrs)) };
    let width = (text_size.width + TEXT_INSET_X + TEXT_INSET_X + WIDTH_SLACK).ceil();
    let size = NSSize::new(width, thickness);
    let y = ((thickness - text_size.height) * 0.5).max(0.0);
    let point = NSPoint::new(TEXT_INSET_X, y);
    let handler = RcBlock::new(move |_: NSRect| -> Bool {
        // SAFETY: `attrs` is a valid string-drawing dictionary; origin is top-left (`flipped`).
        unsafe {
            text.drawAtPoint_withAttributes(point, Some(&attrs));
        }
        Bool::from(true)
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(size, true, &handler);
    image.setTemplate(true);
    image
}
