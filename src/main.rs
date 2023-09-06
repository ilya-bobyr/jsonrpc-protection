use {
    futures_util::future::Either,
    jsonrpc_core::{
        middleware::Middleware,
        types::{
            error::{Error as JsonRpcError, ErrorCode},
            request::{Call, MethodCall},
            response::{Output, Response},
        },
        Id, IoHandlerExtension, MetaIoHandler, Metadata, Version,
    },
    jsonrpc_derive::rpc,
    jsonrpc_http_server::ServerBuilder,
    std::{future::Future, pin::Pin},
    thiserror::Error,
    tokio::runtime,
};

#[derive(Error, Debug, Clone)]
pub enum Error {
    #[error("X-Admin-Auth header value must contain only visible ASCII characters")]
    AdminAuthHeaderParserError,
}

#[derive(Clone)]
pub struct RpcMeta {
    pub auth: Option<Result<String, Error>>,
}
impl Metadata for RpcMeta {}

mod main_rpc {
    use {
        super::{rpc, RpcMeta},
        jsonrpc_core::Result,
    };

    #[rpc]
    pub trait MainRpc {
        type Metadata;

        #[rpc(name = "g")]
        fn g(&self, a: u8, b: u8) -> Result<u8>;
    }

    pub struct MainRpcImpl;
    impl MainRpc for MainRpcImpl {
        type Metadata = RpcMeta;

        fn g(&self, a: u8, b: u8) -> Result<u8> {
            Ok(a.saturating_mul(10).saturating_add(b).saturating_sub(3))
        }
    }
}

mod admin_rpc {
    use {
        super::{rpc, RpcMeta},
        jsonrpc_core::Result,
    };

    #[rpc]
    pub trait AdminRpc {
        type Metadata;

        #[rpc(name = "f")]
        fn f(&self, a: u8, b: u8) -> Result<u8>;
    }

    pub struct AdminRpcImpl;
    impl AdminRpc for AdminRpcImpl {
        type Metadata = RpcMeta;

        fn f(&self, a: u8, b: u8) -> Result<u8> {
            Ok(a.saturating_mul(10).saturating_add(b).saturating_add(2))
        }
    }
}

use admin_rpc::{AdminRpc, AdminRpcImpl};
use main_rpc::{MainRpc, MainRpcImpl};

struct ProtectRpcMiddleware {
    protected: Vec<String>,
}

impl ProtectRpcMiddleware {
    fn new(protected: Vec<String>) -> Self {
        Self { protected }
    }

    fn handle_admin_rpc_call<F, X>(
        &self,
        next: F,
        call: Call,
        meta: RpcMeta,
        jsonrpc: Option<Version>,
        method: String,
        id: Id,
    ) -> Either<Pin<Box<dyn Future<Output = Option<Output>> + Send>>, X>
    where
        F: Fn(Call, RpcMeta) -> X + Send + Sync,
        X: Future<Output = Option<Output>> + Send + 'static,
    {
        type CallFuture = <ProtectRpcMiddleware as Middleware<RpcMeta>>::CallFuture;

        if self.protected.contains(&method) {
            let unauthorized_error = |message| -> Either<CallFuture, X> {
                let error = JsonRpcError {
                    code: ErrorCode::InvalidRequest,
                    message,
                    data: None,
                };
                let id = id.clone();
                let jsonrpc = jsonrpc.clone();

                Either::Left(Box::pin(async move {
                    Some(Output::from(Err(error), id, jsonrpc))
                }))
            };

            let Some(auth) = &meta.auth else {
                return unauthorized_error("X-Admin-Auth header required".to_owned());
            };

            let auth = match auth {
                Ok(auth) => auth,
                Err(error) => return unauthorized_error(error.to_string()),
            };

            if auth != "root" {
                return unauthorized_error("X-Admin-Auth must be 'root'".to_owned());
            }
        }

        Either::Right(next(call, meta))
    }
}

impl Middleware<RpcMeta> for ProtectRpcMiddleware {
    type Future = Pin<Box<dyn Future<Output = Option<Response>> + Send + 'static>>;
    type CallFuture = Pin<Box<dyn Future<Output = Option<Output>> + Send + 'static>>;

    fn on_call<F, X>(&self, call: Call, meta: RpcMeta, next: F) -> Either<Self::CallFuture, X>
    where
        F: Fn(Call, RpcMeta) -> X + Send + Sync,
        X: Future<Output = Option<Output>> + Send + 'static,
    {
        match &call {
            Call::MethodCall(MethodCall {
                jsonrpc,
                method,
                id,
                ..
            }) => {
                let jsonrpc = jsonrpc.clone();
                let method = method.clone();
                let id = id.clone();
                self.handle_admin_rpc_call(next, call, meta, jsonrpc, method, id)
            }
            _ => Either::Right(next(call, meta)),
        }
    }
}

fn main() {
    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let protect_middleware = ProtectRpcMiddleware::new(
        AdminRpcImpl
            .to_delegate()
            .into_iter()
            .map(|(name, _)| name)
            .collect(),
    );

    let mut io = MetaIoHandler::with_middleware(protect_middleware);

    let main_rpc = MainRpcImpl;
    io.extend_with(main_rpc.to_delegate());

    let mut admin_io = MetaIoHandler::default();
    let admin_rpc = AdminRpcImpl;
    admin_io.extend_with(admin_rpc.to_delegate());
    admin_io.augment(&mut io);

    let server =
        ServerBuilder::with_meta_extractor(io, move |req: &hyper::Request<hyper::Body>| {
            let auth = req.headers().get("X-Admin-Auth").map(|v| {
                v.to_str()
                    .map(str::to_owned)
                    .map_err(|_| Error::AdminAuthHeaderParserError)
            });

            RpcMeta { auth }
        })
        .event_loop_executor(rt.handle().clone())
        .start_http(&"0.0.0.0:33481".parse().unwrap())
        .expect("Server must start with no issues");

    server.wait();
}
