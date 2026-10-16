// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use objc2::{
  runtime::{Bool, ProtocolObject},
  ClassType, DeclaredClass,
};
use objc2_app_kit::{NSDragOperation, NSDraggingInfo};
use objc2_foundation::{NSArray, NSPoint, NSURL};

use crate::DragDropEvent;

use super::WryWebView;

pub(crate) fn collect_paths(drag_info: &ProtocolObject<dyn NSDraggingInfo>) -> Vec<PathBuf> {
  let pb = drag_info.draggingPasteboard();
  let classes = NSArray::from_slice(&[NSURL::class()]);
  // Read the current file-URL contract instead of the deprecated filenames
  // property list. Non-file drags have no host paths and never require casts
  // or unwraps on externally supplied pasteboard data.
  let Some(urls) = (unsafe { pb.readObjectsForClasses_options(&classes, None) }) else {
    return Vec::new();
  };
  urls
    .into_iter()
    .filter_map(|object| object.downcast::<NSURL>().ok())
    .filter(|url| url.isFileURL())
    .filter_map(|url| url.path())
    .map(|path| PathBuf::from(path.to_string()))
    .collect()
}

/// AppKit supplies window-relative points; the cross-platform drag event uses
/// physical pixels relative to the webview's top-left corner. Convert through
/// the view so child offsets, flipped bounds and the current display's backing
/// scale are all applied before rounding.
fn drag_position(this: &WryWebView, drag_info: &ProtocolObject<dyn NSDraggingInfo>) -> (i32, i32) {
  let point = this.convertPoint_fromView(drag_info.draggingLocation(), None);
  let bounds = this.bounds();
  let top_left = NSPoint::new(
    bounds.origin.x,
    if this.isFlipped() {
      bounds.origin.y
    } else {
      bounds.origin.y + bounds.size.height
    },
  );
  let point = this.convertPointToBacking(point);
  let top_left = this.convertPointToBacking(top_left);
  // Backing coordinates have an upward Y axis, including for flipped views.
  (
    (point.x - top_left.x).round() as i32,
    (top_left.y - point.y).round() as i32,
  )
}

pub(crate) fn dragging_entered(
  this: &WryWebView,
  drag_info: &ProtocolObject<dyn NSDraggingInfo>,
) -> NSDragOperation {
  let paths = collect_paths(drag_info);
  let position = drag_position(this, drag_info);

  let listener = &this.ivars().drag_drop_handler;
  if !listener(DragDropEvent::Enter { paths, position }) {
    // Reject the Wry file drop (invoke the OS default behaviour)
    unsafe { objc2::msg_send![super(this), draggingEntered: drag_info] }
  } else {
    NSDragOperation::Copy
  }
}

pub(crate) fn dragging_updated(
  this: &WryWebView,
  drag_info: &ProtocolObject<dyn NSDraggingInfo>,
) -> NSDragOperation {
  let position = drag_position(this, drag_info);

  let listener = &this.ivars().drag_drop_handler;
  if !listener(DragDropEvent::Over { position }) {
    unsafe {
      let os_operation = objc2::msg_send![super(this), draggingUpdated: drag_info];
      if os_operation == NSDragOperation::None {
        // 0 will be returned for a drop on any arbitrary location on the webview.
        // We'll override that with NSDragOperationCopy.
        NSDragOperation::Copy
      } else {
        // A different NSDragOperation is returned when a file is hovered over something like
        // a <input type="file">, so we'll make sure to preserve that behaviour.
        os_operation
      }
    }
  } else {
    NSDragOperation::Copy
  }
}

pub(crate) fn perform_drag_operation(
  this: &WryWebView,
  drag_info: &ProtocolObject<dyn NSDraggingInfo>,
) -> Bool {
  let paths = collect_paths(drag_info);
  let position = drag_position(this, drag_info);

  let listener = &this.ivars().drag_drop_handler;
  if !listener(DragDropEvent::Drop { paths, position }) {
    // Reject the Wry drop (invoke the OS default behaviour)
    unsafe { objc2::msg_send![super(this), performDragOperation: drag_info] }
  } else {
    Bool::YES
  }
}

pub(crate) fn dragging_exited(this: &WryWebView, drag_info: &ProtocolObject<dyn NSDraggingInfo>) {
  let listener = &this.ivars().drag_drop_handler;
  if !listener(DragDropEvent::Leave) {
    // Reject the Wry drop (invoke the OS default behaviour)
    unsafe { objc2::msg_send![super(this), draggingExited: drag_info] }
  }
}
