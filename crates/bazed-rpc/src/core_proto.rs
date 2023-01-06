use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::keycode::KeyInput;

/// Id of a request, which is any RPC invocation that expects a response.
#[derive(Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub Uuid);

/// Absolute position within a document
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CaretPosition {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "method", content = "params")]
pub enum ToFrontend {
    OpenDocument {
        document_id: Uuid,
        path: Option<PathBuf>,
        text: String,
    },
    /// Sent whenever anything in the view changed, i.e. the content,
    /// the viewport, or a caret position
    UpdateView {
        view_id: Uuid,
        first_line: usize,
        height: usize,
        text: Vec<String>,
        /// caret positions are absolute
        carets: Vec<CaretPosition>,
        vim_mode: String,
    },
    /// Response to the [ToBackend::ViewOpened] request
    ViewOpenedResponse {
        request_id: RequestId,
        view_id: Uuid,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "method", content = "params")]
pub enum ToBackend {
    SaveDocument {
        document_id: Uuid,
    },
    KeyPressed {
        view_id: Uuid,
        input: KeyInput,
    },
    /// Mouse was clicked notification.
    MouseInput {
        view_id: Uuid,
        /// Absolute coordinates, see [CaretPosition]
        position: CaretPosition,
    },
    /// Mouse wheel turned notification.
    MouseScroll {
        view_id: Uuid,
        /// Positive or negative values mean scrolling down or up respectively
        line_delta: i32,
    },
    /// Send when the viewport for a given view has changed,
    /// i.e. because the window was resized or the user scrolled.
    ViewportChanged {
        view_id: Uuid,
        height: usize,
    },
    ViewOpened {
        request_id: RequestId,
        document_id: Uuid,
        height: usize,
    },
}
