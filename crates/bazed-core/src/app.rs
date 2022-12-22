use std::{collections::HashMap, sync::Arc};

use bazed_rpc::{
    core_proto::ToBackend,
    core_proto::{CaretPosition, ToFrontend},
    keycode::KeyInput,
    server::ClientSendHandle,
};
use color_eyre::Result;
use futures::StreamExt;
use tokio::sync::RwLock;

use crate::{
    document::{Document, DocumentId},
    input_mapper::interpret_key_input,
    user_buffer_op::{DocumentOp, Operation},
    view::{View, ViewId},
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("No document with id {0} found")]
    InvalidDocumentId(DocumentId),
    #[error("No view with id {0} found")]
    InvalidViewId(ViewId),
}

pub struct App {
    documents: HashMap<DocumentId, Document>,
    views: HashMap<ViewId, View>,
    event_send: ClientSendHandle,
}

impl App {
    pub fn new(event_send: ClientSendHandle) -> Self {
        App {
            documents: HashMap::new(),
            event_send,
            views: HashMap::new(),
        }
    }

    async fn open_document(&mut self, document: Document) -> Result<()> {
        let id = DocumentId::gen();
        self.event_send
            .send_rpc(ToFrontend::OpenDocument {
                document_id: id.0,
                path: document.path.clone(),
                text: document.buffer.content_to_string(),
            })
            .await?;
        self.documents.insert(id, document);
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn open_file(&mut self, path: std::path::PathBuf) -> Result<()> {
        let document = Document::open_file(path)?;
        self.open_document(document).await
    }

    #[tracing::instrument(skip(self))]
    async fn open_ephemeral(&mut self) -> Result<()> {
        let document = Document::open_ephemeral();
        self.open_document(document).await
    }

    #[tracing::instrument(skip(self))]
    async fn handle_rpc_call(&mut self, call: ToBackend) -> Result<()> {
        tracing::info!(call = ?call, "Handling rpc call");
        match call {
            ToBackend::KeyPressed { view_id, input } => {
                self.handle_key_pressed(ViewId::from_uuid(view_id), input)
                    .await?
            },

            ToBackend::MouseInput { view_id, position } => {
                self.handle_mouse_input(ViewId::from_uuid(view_id), position)?
            },
            ToBackend::ViewportChanged {
                view_id,
                height,
                width,
                first_line,
                first_col,
            } => {
                self.handle_viewport_changed(
                    ViewId::from_uuid(view_id),
                    height,
                    width,
                    first_line,
                    first_col,
                )
                .await?;
            },
            ToBackend::ViewOpened {
                request_id,
                document_id,
                height,
                width,
            } => {
                let view_id = self
                    .handle_view_opened(DocumentId::from_uuid(document_id), height, width)
                    .await?;
                self.event_send
                    .send_rpc(ToFrontend::ViewOpenedResponse {
                        request_id,
                        view_id: view_id.into(),
                    })
                    .await?;
            },
            ToBackend::SaveDocument { document_id } => {
                self.handle_save_document(DocumentId::from_uuid(document_id))
                    .await?;
            },
        }
        Ok(())
    }

    async fn handle_save_document(&mut self, document_id: DocumentId) -> Result<()> {
        let document = self
            .documents
            .get_mut(&document_id)
            .ok_or(Error::InvalidDocumentId(document_id))?;
        Ok(document.write_to_file().await?)
    }

    async fn handle_viewport_changed(
        &mut self,
        view_id: ViewId,
        height: usize,
        width: usize,
        first_line: usize,
        first_col: usize,
    ) -> Result<()> {
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(Error::InvalidViewId(view_id))?;
        let needs_new_view_info = height > view.height || view.first_line != first_line;
        view.width = width;
        view.height = height;
        view.first_line = first_line;
        view.first_col = first_col;

        if needs_new_view_info {
            let document = self
                .documents
                .get(&view.document_id)
                .ok_or(Error::InvalidDocumentId(view.document_id))?;
            self.event_send
                .send_rpc(document.create_update_notification(view_id, view))
                .await?;
        }
        Ok(())
    }
    async fn handle_key_pressed(&mut self, view_id: ViewId, input: KeyInput) -> Result<()> {
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(Error::InvalidViewId(view_id))?;
        let document = self
            .documents
            .get_mut(&view.document_id)
            .ok_or(Error::InvalidDocumentId(view.document_id))?;

        let Some(operation) = interpret_key_input(&input) else {
            tracing::info!("Ignoring unhandled key input: {input:?}");
            return Ok(())
        };
        match operation {
            Operation::Document(op) => match op {
                DocumentOp::Save => document.write_to_file().await?,
            },
            Operation::Edit(op) => document.buffer.apply_edit_op(op),
            Operation::Movement(op) => document.buffer.apply_movement_op(view, op),
        }
        self.event_send
            .send_rpc(document.create_update_notification(view_id, view))
            .await?;
        Ok(())
    }

    fn handle_mouse_input(&mut self, view: ViewId, coords: CaretPosition) -> Result<()> {
        let _view = self
            .views
            .get_mut(&view)
            .ok_or(Error::InvalidViewId(view))?;
        tracing::info!("mouse input: {coords:?}. No handling implemented so far");
        Ok(())
    }

    async fn handle_view_opened(
        &mut self,
        document_id: DocumentId,
        height: usize,
        width: usize,
    ) -> Result<ViewId> {
        if !self.documents.contains_key(&document_id) {
            return Err(Error::InvalidDocumentId(document_id).into());
        }
        let view = View::new(document_id, height, width);
        let id = ViewId::gen();
        self.views.insert(id, view);
        Ok(id)
    }

    pub fn views(&self) -> &HashMap<ViewId, View> {
        &self.views
    }
}

pub async fn start(addr: &str, path: Option<std::path::PathBuf>) -> Result<()> {
    let (send, mut recv) = bazed_rpc::server::wait_for_client(addr).await?;

    let core = Arc::new(RwLock::new(App::new(send)));

    tokio::spawn({
        let core = core.clone();
        async move {
            while let Some(rpc_call) = recv.next().await {
                let mut core = core.write().await;
                if let Err(err) = core.handle_rpc_call(rpc_call).await {
                    tracing::error!("Failed to handle rpc call: {err:?}");
                }
            }
        }
    });

    if let Some(path) = path {
        core.write().await.open_file(path).await?;
    } else {
        core.write().await.open_ephemeral().await?;
    }

    Ok(())
}
