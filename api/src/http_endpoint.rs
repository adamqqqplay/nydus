// Copyright 2020 Ant Financial. All rights reserved.
// Copyright © 2019 Intel Corporation
//
// SPDX-License-Identifier: Apache-2.0

use std::io;
use std::sync::mpsc::{Receiver, RecvError, SendError, Sender};

use micro_http::{Body, Method, Request, Response, StatusCode, Version};
use serde_json::Error as SerdeError;
use vmm_sys_util::eventfd::EventFd;

use crate::http::{extract_query_part, EndpointHandler};

#[derive(Debug)]
pub enum DaemonErrorKind {
    NotReady,
    NoResource,
    Connect(io::Error),
    SendFd,
    RecvFd,
    Disconnect(io::Error),
    Channel,
    Other,
}

/// API errors are sent back from the VMM API server through the ApiResponse.
#[derive(Debug)]
pub enum ApiError {
    /// Cannot write to EventFd.
    EventFdWrite(io::Error),

    /// Cannot mount a resource
    MountFailure(io::Error),

    /// API request send error
    RequestSend(SendError<ApiRequest>),

    /// Wrong response payload type
    ResponsePayloadType,

    /// API response receive error
    ResponseRecv(RecvError),

    DaemonAbnormal(DaemonErrorKind),
}
pub type ApiResult<T> = std::result::Result<T, ApiError>;

pub enum ApiResponsePayload {
    /// No data is sent on the channel.
    Empty,
    /// Nydus daemon general working information.
    DaemonInfo(DaemonInfo),
    /// Nydus filesystem global metrics
    FsGlobalMetrics(String),
    /// Nydus filesystem per-file metrics
    FsFilesMetrics(String),
    FsFilesPatterns(String),
}

/// This is the response sent by the API server through the mpsc channel.
pub type ApiResponse = std::result::Result<ApiResponsePayload, ApiError>;

//#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum ApiRequest {
    DaemonInfo,
    Mount(MountInfo),
    ConfigureDaemon(DaemonConf),
    ExportGlobalMetrics(Option<String>),
    ExportFilesMetrics(Option<String>),
    ExportAccessPatterns(Option<String>),
    SendFuseFd,
    Takeover,
    Exit,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct DaemonInfo {
    pub id: Option<String>,
    pub version: String,
    pub supervisor: Option<String>,
    pub state: String,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct MountInfo {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub fstype: Option<String>,
    pub mountpoint: String,
    #[serde(default)]
    pub config: Option<String>,
    pub ops: String,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct DaemonConf {
    pub log_level: String,
}

pub fn daemon_info(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    from_api: &Receiver<ApiResponse>,
) -> ApiResult<DaemonInfo> {
    to_api
        .send(ApiRequest::DaemonInfo)
        .map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;

    let info = from_api.recv().map_err(ApiError::ResponseRecv)??;

    match info {
        ApiResponsePayload::DaemonInfo(info) => Ok(info),
        _ => Err(ApiError::ResponsePayloadType),
    }
}

pub fn daemon_configure(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    conf: DaemonConf,
    from_api: &Receiver<ApiResponse>,
) -> ApiResult<()> {
    to_api
        .send(ApiRequest::ConfigureDaemon(conf))
        .map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;

    let info = from_api.recv().map_err(ApiError::ResponseRecv)??;

    match info {
        ApiResponsePayload::Empty => Ok(()),
        _ => Err(ApiError::ResponsePayloadType),
    }
}

pub fn mount_info(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    info: MountInfo,
    from_api: &Receiver<ApiResponse>,
) -> ApiResult<()> {
    // Send the VM request.
    to_api
        .send(ApiRequest::Mount(info))
        .map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;

    let info = from_api.recv().map_err(ApiError::ResponseRecv)??;

    match info {
        ApiResponsePayload::Empty => Ok(()),
        _ => Err(ApiError::ResponsePayloadType),
    }
}

pub fn export_global_stats(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    id: Option<String>,
    from_api: &Receiver<ApiResponse>,
) -> ApiResult<String> {
    to_api
        .send(ApiRequest::ExportGlobalMetrics(id))
        .map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;

    let info = from_api.recv().map_err(ApiError::ResponseRecv)??;

    match info {
        ApiResponsePayload::FsGlobalMetrics(info) => Ok(info),
        _ => Err(ApiError::ResponsePayloadType),
    }
}

pub fn export_files_stats(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    id: Option<String>,
    from_api: &Receiver<ApiResponse>,
) -> ApiResult<String> {
    to_api
        .send(ApiRequest::ExportFilesMetrics(id))
        .map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;

    let info = from_api.recv().map_err(ApiError::ResponseRecv)??;

    match info {
        ApiResponsePayload::FsFilesMetrics(info) => Ok(info),
        _ => Err(ApiError::ResponsePayloadType),
    }
}

pub fn export_files_patterns(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    id: Option<String>,
    from_api: &Receiver<ApiResponse>,
) -> ApiResult<String> {
    to_api
        .send(ApiRequest::ExportAccessPatterns(id))
        .map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;

    let info = from_api.recv().map_err(ApiError::ResponseRecv)??;

    match info {
        ApiResponsePayload::FsFilesPatterns(info) => Ok(info),
        _ => Err(ApiError::ResponsePayloadType),
    }
}

fn send_fuse_fd(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    from_api: &Receiver<ApiResponse>,
) -> ApiResult<()> {
    to_api
        .send(ApiRequest::SendFuseFd)
        .map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;

    let info = from_api.recv().map_err(ApiError::ResponseRecv)??;

    match info {
        ApiResponsePayload::Empty => Ok(()),
        _ => Err(ApiError::ResponsePayloadType),
    }
}

pub fn do_takeover(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    from_api: &Receiver<ApiResponse>,
) -> ApiResult<()> {
    to_api
        .send(ApiRequest::Takeover)
        .map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;

    let info = from_api.recv().map_err(ApiError::ResponseRecv)??;

    match info {
        ApiResponsePayload::Empty => Ok(()),
        _ => Err(ApiError::ResponsePayloadType),
    }
}

pub fn do_exit(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    from_api: &Receiver<ApiResponse>,
) -> ApiResult<()> {
    to_api
        .send(ApiRequest::Exit)
        .map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;

    let info = from_api.recv().map_err(ApiError::ResponseRecv)??;

    match info {
        ApiResponsePayload::Empty => Ok(()),
        _ => Err(ApiError::ResponsePayloadType),
    }
}

/// Errors associated with VMM management
#[derive(Debug)]
pub enum HttpError {
    /// API request receive error
    SerdeJsonDeserialize(SerdeError),
    ParseBody,
    /// Could not query daemon info
    Info(ApiError),
    /// Could not mount resource
    Mount(ApiError),
    Configure(ApiError),
    Upgrade(ApiError),
}

fn error_response(error: HttpError, status: StatusCode) -> Response {
    let mut response = Response::new(Version::Http11, status);
    response.set_body(Body::new(format!("{:?}", error)));
    response
}

// /api/v1/daemon handler
pub struct InfoHandler {}

impl EndpointHandler for InfoHandler {
    fn handle_request(
        &self,
        req: &Request,
        api_notifier: EventFd,
        to_api: Sender<ApiRequest>,
        from_api: &Receiver<ApiResponse>,
    ) -> Response {
        match req.method() {
            Method::Get => {
                match daemon_info(api_notifier, to_api, from_api).map_err(HttpError::Info) {
                    Ok(info) => {
                        let mut response = Response::new(Version::Http11, StatusCode::OK);
                        let info_serialized = serde_json::to_string(&info).unwrap();
                        response.set_body(Body::new(info_serialized));
                        response
                    }
                    Err(e) => error_response(e, StatusCode::InternalServerError),
                }
            }
            Method::Put => match &req.body {
                Some(body) => {
                    let kv: DaemonConf = match serde_json::from_slice(body.raw())
                        .map_err(HttpError::SerdeJsonDeserialize)
                    {
                        Ok(config) => config,
                        Err(e) => return error_response(e, StatusCode::BadRequest),
                    };

                    match daemon_configure(api_notifier, to_api, kv, from_api)
                        .map_err(HttpError::Configure)
                    {
                        Ok(()) => Response::new(Version::Http11, StatusCode::NoContent),
                        Err(e) => error_response(e, StatusCode::InternalServerError),
                    }
                }
                None => Response::new(Version::Http11, StatusCode::BadRequest),
            },
            _ => Response::new(Version::Http11, StatusCode::BadRequest),
        }
    }
}

fn kick_api_server(
    api_evt: EventFd,
    to_api: Sender<ApiRequest>,
    from_api: &Receiver<ApiResponse>,
    request: ApiRequest,
) -> ApiResponse {
    to_api.send(request).map_err(ApiError::RequestSend)?;
    api_evt.write(1).map_err(ApiError::EventFdWrite)?;
    from_api.recv().map_err(ApiError::ResponseRecv)?
}

fn parse_mount_request(body: &Body) -> Result<MountInfo, HttpError> {
    serde_json::from_slice::<MountInfo>(body.raw()).map_err(|_| HttpError::ParseBody)
}

fn success_response(body: Option<String>) -> Response {
    let mut r = Response::new(Version::Http11, StatusCode::NoContent);
    if let Some(b) = body {
        r.set_body(Body::new(b));
    }
    r
}

fn convert_to_response<O: FnOnce(ApiError) -> HttpError>(api_resp: ApiResponse, op: O) -> Response {
    match api_resp {
        Ok(r) => match r {
            ApiResponsePayload::Empty => success_response(None),
            ApiResponsePayload::DaemonInfo(d) => {
                success_response(Some(serde_json::to_string(&d).unwrap()))
            }
            ApiResponsePayload::FsFilesMetrics(d) => {
                success_response(Some(serde_json::to_string(&d).unwrap()))
            }
            ApiResponsePayload::FsGlobalMetrics(d) => {
                success_response(Some(serde_json::to_string(&d).unwrap()))
            }
            ApiResponsePayload::FsFilesPatterns(d) => {
                success_response(Some(serde_json::to_string(&d).unwrap()))
            }
        },
        Err(e) => error_response(op(e), StatusCode::InternalServerError),
    }
}

// /api/v1/mount handler
pub struct MountHandler {}

impl EndpointHandler for MountHandler {
    fn handle_request(
        &self,
        req: &Request,
        api_notifier: EventFd,
        to_api: Sender<ApiRequest>,
        from_api: &Receiver<ApiResponse>,
    ) -> Response {
        match (req.method(), req.body.as_ref()) {
            (Method::Put, Some(body)) => {
                let info = parse_mount_request(body).unwrap();
                let r = kick_api_server(api_notifier, to_api, from_api, ApiRequest::Mount(info));
                convert_to_response(r, HttpError::Mount)
            }
            _ => Response::new(Version::Http11, StatusCode::BadRequest),
        }
    }
}

pub struct MetricsHandler {}

impl EndpointHandler for MetricsHandler {
    fn handle_request(
        &self,
        req: &Request,
        api_notifier: EventFd,
        to_api: Sender<ApiRequest>,
        from_api: &Receiver<ApiResponse>,
    ) -> Response {
        match req.method() {
            Method::Get => {
                let id = extract_query_part(req, &"id");
                match export_global_stats(api_notifier, to_api, id, from_api)
                    .map_err(HttpError::Info)
                {
                    Ok(info) => {
                        let mut response = Response::new(Version::Http11, StatusCode::OK);
                        response.set_body(Body::new(info));
                        response
                    }
                    Err(e) => error_response(e, StatusCode::InternalServerError),
                }
            }
            _ => Response::new(Version::Http11, StatusCode::BadRequest),
        }
    }
}

pub struct MetricsFilesHandler {}

impl EndpointHandler for MetricsFilesHandler {
    fn handle_request(
        &self,
        req: &Request,
        api_notifier: EventFd,
        to_api: Sender<ApiRequest>,
        from_api: &Receiver<ApiResponse>,
    ) -> Response {
        match req.method() {
            Method::Get => {
                let id = extract_query_part(req, &"id");
                match export_files_stats(api_notifier, to_api, id, from_api)
                    .map_err(HttpError::Info)
                {
                    Ok(info) => {
                        let mut response = Response::new(Version::Http11, StatusCode::OK);
                        response.set_body(Body::new(info));
                        response
                    }
                    Err(e) => error_response(e, StatusCode::InternalServerError),
                }
            }
            _ => Response::new(Version::Http11, StatusCode::BadRequest),
        }
    }
}

pub struct MetricsPatternHandler {}

impl EndpointHandler for MetricsPatternHandler {
    fn handle_request(
        &self,
        req: &Request,
        api_notifier: EventFd,
        to_api: Sender<ApiRequest>,
        from_api: &Receiver<ApiResponse>,
    ) -> Response {
        match req.method() {
            Method::Get => {
                let id = extract_query_part(req, &"id");
                match export_files_patterns(api_notifier, to_api, id, from_api)
                    .map_err(HttpError::Info)
                {
                    Ok(info) => {
                        let mut response = Response::new(Version::Http11, StatusCode::OK);
                        response.set_body(Body::new(info));
                        response
                    }
                    Err(e) => error_response(e, StatusCode::InternalServerError),
                }
            }
            _ => Response::new(Version::Http11, StatusCode::BadRequest),
        }
    }
}

pub struct SendFuseFdHandler {}

impl EndpointHandler for SendFuseFdHandler {
    fn handle_request(
        &self,
        req: &Request,
        api_notifier: EventFd,
        to_api: Sender<ApiRequest>,
        from_api: &Receiver<ApiResponse>,
    ) -> Response {
        match req.method() {
            Method::Put => match send_fuse_fd(api_notifier, to_api, from_api) {
                Ok(_) => Response::new(Version::Http11, StatusCode::NoContent),
                Err(e) => match e {
                    ApiError::DaemonAbnormal(DaemonErrorKind::NotReady) => {
                        error_response(HttpError::Upgrade(e), StatusCode::BadRequest)
                    }
                    _ => error_response(HttpError::Upgrade(e), StatusCode::InternalServerError),
                },
            },
            _ => Response::new(Version::Http11, StatusCode::BadRequest),
        }
    }
}

pub struct TakeoverHandler {}

impl EndpointHandler for TakeoverHandler {
    fn handle_request(
        &self,
        req: &Request,
        api_notifier: EventFd,
        to_api: Sender<ApiRequest>,
        from_api: &Receiver<ApiResponse>,
    ) -> Response {
        match req.method() {
            Method::Put => {
                match do_takeover(api_notifier, to_api, from_api).map_err(HttpError::Upgrade) {
                    Ok(_) => Response::new(Version::Http11, StatusCode::NoContent),
                    Err(e) => error_response(e, StatusCode::InternalServerError),
                }
            }
            _ => Response::new(Version::Http11, StatusCode::BadRequest),
        }
    }
}

pub struct ExitHandler {}

impl EndpointHandler for ExitHandler {
    fn handle_request(
        &self,
        req: &Request,
        api_notifier: EventFd,
        to_api: Sender<ApiRequest>,
        from_api: &Receiver<ApiResponse>,
    ) -> Response {
        match req.method() {
            Method::Put => {
                match do_exit(api_notifier, to_api, from_api).map_err(HttpError::Upgrade) {
                    Ok(_) => Response::new(Version::Http11, StatusCode::NoContent),
                    Err(e) => error_response(e, StatusCode::InternalServerError),
                }
            }
            _ => Response::new(Version::Http11, StatusCode::BadRequest),
        }
    }
}
