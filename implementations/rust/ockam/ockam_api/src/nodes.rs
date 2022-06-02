pub mod types;

use crate::{Error, Method, Request, RequestBuilder, Response, Status};
use core::borrow::Borrow;
use core::convert::Infallible;
use core::fmt;
use minicbor::encode;
use minicbor::{Decoder, Encode};
use ockam_core::compat::collections::HashMap;
use ockam_core::compat::rand;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::{self, Address, Cbor, Route, Routed, Worker};
use ockam_node::Context;
use tracing::{trace, warn};
use types::{CreateNode, NodeInfo};

#[derive(Debug, Default)]
pub struct Server(HashMap<String, NodeInfo<'static>>);

#[ockam_core::worker]
impl Worker for Server {
    type Context = Context;
    type Message = Cbor;

    async fn handle_message(
        &mut self,
        ctx: &mut Context,
        msg: Routed<Self::Message>,
    ) -> ockam_core::Result<()> {
        let cbor = self.on_request(msg.as_body())?;
        ctx.send(msg.return_route(), cbor).await
    }
}

impl Server {
    pub fn new() -> Self {
        Server::default()
    }

    fn on_request(&mut self, data: &Cbor) -> Result<Cbor, NodesError> {
        let mut dec = Decoder::new(data.borrow());
        let req: Request = dec.decode()?;

        trace! {
            target: "ockam_api::nodes::server",
            id     = %req.id(),
            method = ?req.method(),
            path   = %req.path(),
            body   = %req.has_body(),
            "request"
        }

        let res = match req.method() {
            Some(Method::Get) => match req.path_segments::<2>().as_slice() {
                // Get all nodes:
                [""] => Response::ok(req.id())
                    .body(encode::ArrayIter::new(self.0.values()))
                    .to_cbor()?,
                // Get a single node:
                [id] => {
                    if let Some(n) = self.0.get(*id) {
                        Response::ok(req.id()).body(n).to_cbor()?
                    } else {
                        Response::not_found(req.id()).to_cbor()?
                    }
                }
                _ => {
                    let error = Error::new(req.path())
                        .with_method(Method::Post)
                        .with_message("unknown path");
                    Response::bad_request(req.id()).body(error).to_cbor()?
                }
            },
            Some(Method::Post) if req.has_body() => {
                let cn = dec.decode::<CreateNode>()?;
                // TODO: replace placeholder:
                let ni = NodeInfo::new()
                    .with_name(cn.name().to_string())
                    .with_id(rand_id());
                let res = Response::ok(req.id()).body(&ni).to_cbor()?;
                self.0.insert(ni.id().to_string(), ni);
                res
            }
            Some(Method::Post) => {
                let error = Error::new(req.path())
                    .with_method(Method::Post)
                    .with_message("missing request body");
                Response::bad_request(req.id()).body(error).to_cbor()?
            }
            Some(Method::Delete) => match req.path_segments::<2>().as_slice() {
                [id] => {
                    if self.0.remove(*id).is_some() {
                        Response::ok(req.id()).to_cbor()?
                    } else {
                        Response::not_found(req.id()).to_cbor()?
                    }
                }
                _ => {
                    let error = Error::new(req.path())
                        .with_method(Method::Post)
                        .with_message("unknown path");
                    Response::bad_request(req.id()).body(error).to_cbor()?
                }
            },
            Some(m) => {
                let error = Error::new(req.path()).with_method(m);
                Response::builder(req.id(), Status::MethodNotAllowed)
                    .body(error)
                    .to_cbor()?
            }
            None => {
                let error = Error::new(req.path()).with_message("unknown method");
                Response::not_implemented(req.id()).body(error).to_cbor()?
            }
        };

        Ok(res)
    }
}

/// TODO: replace placeholder:
fn rand_id() -> String {
    use rand::distributions::{Alphanumeric, DistString};
    Alphanumeric.sample_string(&mut rand::thread_rng(), 16)
}

pub struct Client {
    ctx: Context,
    route: Route,
    buf: Cbor,
}

impl Client {
    pub async fn new(r: Route, ctx: &Context) -> ockam_core::Result<Self> {
        let ctx = ctx.new_detached(Address::random_local()).await?;
        Ok(Client {
            ctx,
            route: r,
            buf: Cbor::default(),
        })
    }

    /// Create a node by name.
    pub async fn create_node(&mut self, body: &CreateNode<'_>) -> ockam_core::Result<NodeInfo<'_>> {
        let req = Request::post("/").body(body);
        trace!(target: "ockam_api::nodes::client", id = %req.header().id(), name = %body.name(), "creating new node");
        self.buf = self.request("create-node", &req).await?;
        let mut d = Decoder::new(&self.buf);
        let res = response("create-node", &mut d)?;
        if res.status() == Some(Status::Ok) {
            d.decode().map_err(|e| e.into())
        } else {
            Err(error("create-node", &res, &mut d))
        }
    }

    /// Get information about a node.
    pub async fn get(&mut self, id: &str) -> ockam_core::Result<Option<NodeInfo<'_>>> {
        let req = Request::get(format!("/{id}"));
        trace!(target: "ockam_api::nodes::client", id = %req.header().id(), node = %id, "getting node info");
        self.buf = self.request("get-node", &req).await?;
        let mut d = Decoder::new(&self.buf);
        let res = response("get-node", &mut d)?;
        match res.status() {
            Some(Status::Ok) => d.decode().map_err(|e| e.into()),
            Some(Status::NotFound) => Ok(None),
            _ => Err(error("get-node", &res, &mut d)),
        }
    }

    /// List all available nodes.
    pub async fn list(&mut self) -> ockam_core::Result<Vec<NodeInfo<'_>>> {
        let req = Request::get("/");
        trace!(target: "ockam_api::nodes::client", id = %req.header().id(), "listing all nodes");
        self.buf = self.request("list-nodes", &req).await?;
        let mut d = Decoder::new(&self.buf);
        let res = response("list-nodes", &mut d)?;
        if res.status() == Some(Status::Ok) {
            d.decode().map_err(|e| e.into())
        } else {
            Err(error("list-nodes", &res, &mut d))
        }
    }

    /// Delete a node.
    pub async fn delete(&mut self, id: &str) -> ockam_core::Result<()> {
        let req = Request::delete(format!("/{id}"));
        trace!(target: "ockam_api::nodes::client", id = %req.header().id(), node = %id, "deleting node");
        self.buf = self.request("delete-node", &req).await?;
        let mut d = Decoder::new(&self.buf);
        let res = response("delete-node", &mut d)?;
        if res.status() == Some(Status::Ok) {
            return Ok(());
        }
        Err(error("delete-node", &res, &mut d))
    }

    /// Encode request header and body (if any) and send the package to the server.
    async fn request<T>(
        &mut self,
        label: &str,
        req: &RequestBuilder<'_, T>,
    ) -> ockam_core::Result<Cbor>
    where
        T: Encode<()>,
    {
        let mut buf = Cbor::default();
        req.encode(&mut buf)?;
        trace!(target: "ockam_api::nodes::client", label = %label, id = %req.header().id(), "-> req");
        let vec = self.ctx.send_and_receive(self.route.clone(), buf).await?;
        Ok(vec)
    }
}

/// Decode and log response header.
fn response(label: &str, dec: &mut Decoder<'_>) -> ockam_core::Result<Response> {
    let res: Response = dec.decode()?;
    trace! {
        target: "ockam_api::nodes::client",
        label  = %label,
        id     = %res.id(),
        re     = %res.re(),
        status = ?res.status(),
        body   = %res.has_body(),
        "<- res"
    }
    Ok(res)
}

/// Decode, log and mape response error to ockam_core error.
fn error(label: &str, res: &Response, dec: &mut Decoder<'_>) -> ockam_core::Error {
    if res.has_body() {
        let err = match dec.decode::<Error>() {
            Ok(e) => e,
            Err(e) => return e.into(),
        };
        warn! {
            target: "ockam_api::nodes::client",
            label  = %label,
            id     = %res.id(),
            re     = %res.re(),
            status = ?res.status(),
            error  = ?err.message(),
            "<- err"
        }
        let msg = err.message().unwrap_or(label);
        ockam_core::Error::new(Origin::Application, Kind::Protocol, msg)
    } else {
        ockam_core::Error::new(Origin::Application, Kind::Protocol, label)
    }
}

/// Potential node errors.
#[derive(Debug)]
pub struct NodesError(ErrorImpl);

#[derive(Debug)]
enum ErrorImpl {
    Decode(minicbor::decode::Error),
    Encode(minicbor::encode::Error<Infallible>),
}

impl fmt::Display for NodesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorImpl::Encode(e) => e.fmt(f),
            ErrorImpl::Decode(e) => e.fmt(f),
        }
    }
}

impl ockam_core::compat::error::Error for NodesError {
    #[cfg(feature = "std")]
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            ErrorImpl::Decode(e) => Some(e),
            ErrorImpl::Encode(e) => Some(e),
        }
    }
}

impl From<minicbor::decode::Error> for NodesError {
    fn from(e: minicbor::decode::Error) -> Self {
        NodesError(ErrorImpl::Decode(e))
    }
}

impl From<minicbor::encode::Error<Infallible>> for NodesError {
    fn from(e: minicbor::encode::Error<Infallible>) -> Self {
        NodesError(ErrorImpl::Encode(e))
    }
}

impl From<NodesError> for ockam_core::Error {
    fn from(e: NodesError) -> Self {
        ockam_core::Error::new(Origin::Application, Kind::Invalid, e)
    }
}
