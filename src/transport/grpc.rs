use std::{
    collections::HashMap,
    fmt::Display,
    path::{Component, PathBuf},
    str::FromStr,
    time::Instant,
};

use anyhow::{bail, Context, Result};
use http::uri::PathAndQuery;
use log::{info, log_enabled, Level};
use prost::Message;
use prost_reflect::{
    Cardinality, DescriptorPool, DynamicMessage, FieldDescriptor, Kind, MessageDescriptor,
    MethodDescriptor, SerializeOptions, Value as ReflectValue,
};
use serde::Serialize;
use serde_json::Value;
use tonic::{
    codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder},
    metadata::{AsciiMetadataKey, MetadataValue},
    transport::{ClientTlsConfig, Endpoint},
    Request, Status,
};

use crate::{http_file, resolve::Resolved, util::truncate};

const PROTO_HEADER: &str = "proto";
const PROTOSET_HEADER: &str = "protoset";

#[derive(Clone, Debug, PartialEq)]
pub enum DescriptorSource {
    Proto(PathBuf),
    Protoset(PathBuf),
}

#[derive(Clone)]
pub struct GrpcRequest {
    pub endpoint: String,
    pub path: String,
    pub descriptor_source: DescriptorSource,
    pub metadata: Vec<(String, String)>,
    pub body: String,
}

impl Display for GrpcRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "GRPC {}{}", self.endpoint, self.path)?;
        match &self.descriptor_source {
            DescriptorSource::Proto(path) => writeln!(f, "Proto: {}", path.display())?,
            DescriptorSource::Protoset(path) => writeln!(f, "Protoset: {}", path.display())?,
        }
        for (key, val) in &self.metadata {
            writeln!(f, "{key}: {val}")?;
        }

        if !self.body.trim().is_empty() {
            writeln!(f)?;
            for line in self.body.lines() {
                writeln!(f, "{line}")?;
            }
        }

        Ok(())
    }
}

pub fn is_grpc_request(rendered: &str) -> bool {
    http_file::parse(rendered)
        .request
        .is_some_and(|request| request.line.method.text == "GRPC")
}

pub fn prepare_request(resolved: &Resolved, rendered: &str) -> Result<GrpcRequest> {
    let document = http_file::parse(rendered);
    if let Some(error) = document.errors.first() {
        bail!("{}", error.message);
    }
    let request = document
        .request
        .context("Invalid gRPC request: missing request line")?;
    if request.line.method.text != "GRPC" {
        bail!("Invalid gRPC request: request must start with GRPC");
    }
    let target = request.line.target.map_or("", |target| target.text);

    if target.is_empty() {
        bail!("Invalid gRPC request: missing target");
    }

    let (endpoint, path) = parse_target(target)?;

    let mut descriptor_source = None;
    let mut metadata = Vec::new();

    for header in request.headers {
        let key = header.name.text;
        let value = header.value.text;

        if key.eq_ignore_ascii_case(PROTO_HEADER) {
            if descriptor_source.is_some() {
                bail!("Invalid gRPC request: specify only one of Proto or Protoset");
            }
            descriptor_source = Some(DescriptorSource::Proto(resolve_file_path(resolved, value)));
        } else if key.eq_ignore_ascii_case(PROTOSET_HEADER) {
            if descriptor_source.is_some() {
                bail!("Invalid gRPC request: specify only one of Proto or Protoset");
            }
            descriptor_source = Some(DescriptorSource::Protoset(resolve_file_path(
                resolved, value,
            )));
        } else {
            metadata.push((key.to_ascii_lowercase(), value.to_string()));
        }
    }

    let descriptor_source = descriptor_source.context(
        "Invalid gRPC request: missing Proto or Protoset header pointing to descriptors",
    )?;

    Ok(GrpcRequest {
        endpoint,
        path,
        descriptor_source,
        metadata,
        body: request
            .body
            .map_or_else(String::new, |body| body.text.to_string()),
    })
}

pub async fn send(req: &GrpcRequest) -> Result<(Value, std::time::Duration)> {
    let method = load_method(req)?;
    let request_desc = method.input();
    let response_desc = method.output();

    let body = if req.body.trim().is_empty() {
        "{}"
    } else {
        req.body.trim()
    };
    let mut deserializer = serde_json::Deserializer::from_str(body);
    let message = DynamicMessage::deserialize(request_desc, &mut deserializer)?;
    deserializer.end()?;

    let endpoint = build_endpoint(&req.endpoint)?;
    let channel = endpoint.connect().await?;
    let mut client = tonic::client::Grpc::new(channel);
    client
        .ready()
        .await
        .map_err(|err| anyhow::anyhow!("gRPC service was not ready: {err}"))?;

    let path = PathAndQuery::from_str(&req.path)?;
    let mut request = Request::new(message);
    for (key, value) in &req.metadata {
        let key = AsciiMetadataKey::from_str(key)?;
        let value = MetadataValue::try_from(value.as_str())?;
        request.metadata_mut().insert(key, value);
    }

    let t = Instant::now();
    let response = client
        .unary(request, path, DynamicCodec::new(response_desc))
        .await?;
    let elapsed = t.elapsed();

    let response_message = response.into_inner();
    let json = serialize_message(&response_message)?;

    Ok((json, elapsed))
}

fn build_endpoint(endpoint: &str) -> Result<Endpoint> {
    let endpoint = Endpoint::from_shared(endpoint.to_string())?;

    if endpoint.uri().scheme_str() == Some("https") {
        Ok(endpoint.tls_config(ClientTlsConfig::new().with_native_roots())?)
    } else {
        Ok(endpoint)
    }
}

pub fn finish_response(json: &Value) -> Result<Option<Value>> {
    println!("{}", serde_json::to_string_pretty(json)?);
    Ok(Some(json.clone()))
}

struct DynamicCodec {
    response: MessageDescriptor,
}

impl DynamicCodec {
    fn new(response: MessageDescriptor) -> Self {
        Self { response }
    }
}

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder {
            response: self.response.clone(),
        }
    }
}

struct DynamicEncoder;

impl Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(dst)
            .map_err(|err| Status::internal(err.to_string()))
    }
}

struct DynamicDecoder {
    response: MessageDescriptor,
}

impl Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        DynamicMessage::decode(self.response.clone(), src)
            .map(Some)
            .map_err(|err| Status::internal(err.to_string()))
    }
}

pub fn print_request(req: &GrpcRequest) {
    if log_enabled!(Level::Info) {
        for line in req.to_string().lines() {
            info!("> {}", truncate(line));
        }
    }
}

fn parse_target(target: &str) -> Result<(String, String)> {
    let normalized = if let Some(rest) = target.strip_prefix("grpc://") {
        format!("http://{rest}")
    } else if let Some(rest) = target.strip_prefix("grpcs://") {
        format!("https://{rest}")
    } else if target.starts_with("http://") || target.starts_with("https://") {
        target.to_string()
    } else {
        format!("http://{target}")
    };

    let uri = http::Uri::from_str(&normalized)?;
    let scheme = uri
        .scheme_str()
        .context("Invalid gRPC target: missing scheme")?;
    let authority = uri
        .authority()
        .context("Invalid gRPC target: missing host")?;
    let path = uri.path();

    if path == "/" || path.is_empty() {
        bail!("Invalid gRPC target: missing /package.Service/Method path");
    }

    Ok((format!("{scheme}://{authority}"), path.to_string()))
}

fn resolve_file_path(resolved: &Resolved, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        normalize_path(
            resolved
                .original_path()
                .parent()
                .unwrap_or(&resolved.root_dir)
                .join(path),
        )
    }
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();

    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            _ => out.push(component.as_os_str()),
        }
    }

    out
}

fn load_method(req: &GrpcRequest) -> Result<MethodDescriptor> {
    let pool = load_descriptor_pool(&req.descriptor_source)?;

    let (service_name, method_name) = req
        .path
        .trim_start_matches('/')
        .rsplit_once('/')
        .context("Invalid gRPC path: expected /package.Service/Method")?;
    let service = pool
        .get_service_by_name(service_name)
        .with_context(|| format!("gRPC service not found: {service_name}"))?;
    let method = service
        .methods()
        .find(|method| method.name() == method_name)
        .with_context(|| format!("gRPC method not found: {method_name}"))?;

    Ok(method)
}

pub fn load_descriptor_pool(source: &DescriptorSource) -> Result<DescriptorPool> {
    let descriptors = match source {
        DescriptorSource::Proto(proto_path) => {
            let import_path = proto_path
                .parent()
                .context("Proto file must have a parent directory")?;
            protox::compile([proto_path.as_path()], [import_path])?
        }
        DescriptorSource::Protoset(protoset_path) => {
            let bytes = std::fs::read(protoset_path).with_context(|| {
                format!("Failed to read protoset file {}", protoset_path.display())
            })?;
            prost_types::FileDescriptorSet::decode(bytes.as_slice())?
        }
    };

    Ok(DescriptorPool::from_file_descriptor_set(descriptors)?)
}

pub fn list_services(source: &DescriptorSource) -> Result<Vec<String>> {
    let pool = load_descriptor_pool(source)?;
    Ok(pool
        .services()
        .map(|service| service.full_name().to_string())
        .collect())
}

pub fn list_methods(
    source: &DescriptorSource,
    service_name: &str,
) -> Result<Vec<MethodDescriptor>> {
    let pool = load_descriptor_pool(source)?;
    let service = pool
        .get_service_by_name(service_name)
        .with_context(|| format!("gRPC service not found: {service_name}"))?;

    Ok(service.methods().collect())
}

pub fn message_template(desc: &MessageDescriptor) -> Result<Value> {
    let message = build_template_message(desc, &HashMap::new());
    serialize_message_with_options(
        &message,
        &SerializeOptions::new().skip_default_fields(false),
    )
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MessageTemplateChoice {
    pub oneof: String,
    pub oneof_label: String,
    pub field: String,
    pub field_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MessageTemplateChoiceGroup {
    pub oneof: String,
    pub oneof_label: String,
    pub options: Vec<MessageTemplateChoice>,
}

pub fn next_message_template_choice(
    desc: &MessageDescriptor,
    choices: &HashMap<String, String>,
) -> Option<MessageTemplateChoiceGroup> {
    next_template_choice(desc, choices)
}

pub fn message_template_with_choices(
    desc: &MessageDescriptor,
    choices: &HashMap<String, String>,
) -> Result<Value> {
    let message = build_template_message(desc, choices);
    serialize_message_with_options(
        &message,
        &SerializeOptions::new().skip_default_fields(false),
    )
}

fn build_template_message(
    desc: &MessageDescriptor,
    choices: &HashMap<String, String>,
) -> DynamicMessage {
    let mut message = DynamicMessage::new(desc.clone());

    for field in desc.fields() {
        if !should_include_template_field(&field, choices) {
            continue;
        }
        let value = template_field_value(&field, choices);
        message.set_field(&field, value);
    }

    message
}

fn should_include_template_field(
    field: &FieldDescriptor,
    choices: &HashMap<String, String>,
) -> bool {
    let Some(oneof) = field.containing_oneof() else {
        return true;
    };
    if oneof.is_synthetic() {
        return true;
    }

    let selected = choices
        .get(oneof.full_name())
        .cloned()
        .or_else(|| oneof.fields().next().map(|field| field.name().to_string()));

    selected.as_deref() == Some(field.name())
}

fn next_template_choice(
    desc: &MessageDescriptor,
    choices: &HashMap<String, String>,
) -> Option<MessageTemplateChoiceGroup> {
    let mut seen_oneofs = Vec::new();

    for field in desc.fields() {
        if let Some(oneof) = field.containing_oneof() {
            if oneof.is_synthetic() || seen_oneofs.iter().any(|seen| seen == oneof.full_name()) {
                continue;
            }
            seen_oneofs.push(oneof.full_name().to_string());

            if oneof.fields().len() > 1 && !choices.contains_key(oneof.full_name()) {
                return Some(MessageTemplateChoiceGroup {
                    oneof: oneof.full_name().to_string(),
                    oneof_label: oneof.name().to_string(),
                    options: oneof
                        .fields()
                        .map(|field| MessageTemplateChoice {
                            oneof: oneof.full_name().to_string(),
                            oneof_label: oneof.name().to_string(),
                            field: field.name().to_string(),
                            field_label: field.json_name().to_string(),
                        })
                        .collect(),
                });
            }

            if !should_include_template_field(&field, choices) {
                continue;
            }
        }

        let Kind::Message(message) = field.kind() else {
            continue;
        };
        if let Some(choice) = next_template_choice(&message, choices) {
            return Some(choice);
        }
    }

    None
}

fn template_field_value(
    field: &FieldDescriptor,
    choices: &HashMap<String, String>,
) -> ReflectValue {
    if field.is_map() {
        return ReflectValue::Map(Default::default());
    }

    if field.cardinality() == Cardinality::Repeated {
        return ReflectValue::List(vec![template_singular_field_value(field, choices)]);
    }

    template_singular_field_value(field, choices)
}

fn template_singular_field_value(
    field: &FieldDescriptor,
    choices: &HashMap<String, String>,
) -> ReflectValue {
    match field.kind() {
        Kind::Double => ReflectValue::F64(0.0),
        Kind::Float => ReflectValue::F32(0.0),
        Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 => ReflectValue::I32(0),
        Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 => ReflectValue::I64(0),
        Kind::Uint32 | Kind::Fixed32 => ReflectValue::U32(0),
        Kind::Uint64 | Kind::Fixed64 => ReflectValue::U64(0),
        Kind::Bool => ReflectValue::Bool(false),
        Kind::String => ReflectValue::String(String::new()),
        Kind::Bytes => ReflectValue::Bytes(Vec::new().into()),
        Kind::Message(message) => ReflectValue::Message(build_template_message(&message, choices)),
        Kind::Enum(en) => ReflectValue::EnumNumber(
            en.values()
                .next()
                .map(|value| value.number())
                .unwrap_or_default(),
        ),
    }
}

fn serialize_message(message: &DynamicMessage) -> Result<Value> {
    serialize_message_with_options(message, &SerializeOptions::new())
}

fn serialize_message_with_options(
    message: &DynamicMessage,
    options: &SerializeOptions,
) -> Result<Value> {
    let mut buf = Vec::new();
    let mut serializer = serde_json::Serializer::new(&mut buf);
    message.serialize_with_options(&mut serializer, options)?;
    Ok(serde_json::from_slice(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::{Resolved, ResolvedAs};
    use mktemp::Temp;
    use std::fs;

    fn resolved(path: &str) -> Resolved {
        Resolved {
            root_dir: PathBuf::from("/tmp/project").into_boxed_path(),
            resolved_as: ResolvedAs::Simple {
                path: PathBuf::from(path).into_boxed_path(),
            },
        }
    }

    #[test]
    fn detects_grpc_request() {
        assert!(is_grpc_request("GRPC localhost:50051/foo.Bar/Baz\n"));
        assert!(is_grpc_request(
            "# @name Call Baz\n# note\nGRPC localhost:50051/foo.Bar/Baz\n"
        ));
        assert!(!is_grpc_request("GET http://example.com\n"));
    }

    #[test]
    fn parses_comments_in_grpc_head_but_not_body() {
        let request = prepare_request(
            &resolved("/tmp/project/call.http"),
            "# @name Call Baz\nGRPC localhost:50051/foo.Bar/Baz\n# descriptors\nProto: ./baz.proto # note\n\n{\"text\":\"# unchanged\"}\n",
        )
        .unwrap();
        assert_eq!(
            request.descriptor_source,
            DescriptorSource::Proto(PathBuf::from("/tmp/project/baz.proto"))
        );
        assert_eq!(request.body, "{\"text\":\"# unchanged\"}\n");
    }

    #[test]
    fn parses_grpc_request() {
        let request = prepare_request(
            &resolved("/tmp/project/requests/get_user.http"),
            r#"GRPC localhost:50051/example.UserService/GetUser
Proto: ../proto/user.proto
Authorization: Bearer abc

{"id":"123"}
"#,
        )
        .unwrap();

        assert_eq!(request.endpoint, "http://localhost:50051");
        assert_eq!(request.path, "/example.UserService/GetUser");
        assert_eq!(
            request.descriptor_source,
            DescriptorSource::Proto(PathBuf::from("/tmp/project/proto/user.proto"))
        );
        assert_eq!(
            request.metadata,
            vec![("authorization".into(), "Bearer abc".into())]
        );
        assert_eq!(request.body.trim(), r#"{"id":"123"}"#);
    }

    #[test]
    fn parses_grpcs_target() {
        let (endpoint, path) = parse_target("grpcs://example.com/package.Service/Method").unwrap();

        assert_eq!(endpoint, "https://example.com");
        assert_eq!(path, "/package.Service/Method");
    }

    #[test]
    fn loads_method_from_proto() {
        let tmp = Temp::new_dir().unwrap();
        let proto = tmp.join("user.proto");
        fs::write(
            &proto,
            r#"
                syntax = "proto3";
                package example;

                service UserService {
                    rpc GetUser (GetUserRequest) returns (GetUserResponse);
                }

                message GetUserRequest {
                    string id = 1;
                }

                message GetUserResponse {
                    string name = 1;
                }
            "#,
        )
        .unwrap();

        let request = GrpcRequest {
            endpoint: "http://localhost:50051".into(),
            path: "/example.UserService/GetUser".into(),
            descriptor_source: DescriptorSource::Proto(proto),
            metadata: vec![],
            body: r#"{"id":"123"}"#.into(),
        };

        let method = load_method(&request).unwrap();

        assert_eq!(method.name(), "GetUser");
        assert_eq!(method.input().full_name(), "example.GetUserRequest");
        assert_eq!(method.output().full_name(), "example.GetUserResponse");
    }

    #[test]
    fn loads_method_from_protoset() {
        let tmp = Temp::new_dir().unwrap();
        let proto = tmp.join("user.proto");
        fs::write(
            &proto,
            r#"
                syntax = "proto3";
                package example;

                service UserService {
                    rpc GetUser (GetUserRequest) returns (GetUserResponse);
                }

                message GetUserRequest {
                    string id = 1;
                }

                message GetUserResponse {
                    string name = 1;
                }
            "#,
        )
        .unwrap();

        let descriptors = protox::compile([proto.as_path()], [tmp.as_path()]).unwrap();
        let protoset = tmp.join("protoset.bin");
        fs::write(&protoset, descriptors.encode_to_vec()).unwrap();

        let request = GrpcRequest {
            endpoint: "http://localhost:50051".into(),
            path: "/example.UserService/GetUser".into(),
            descriptor_source: DescriptorSource::Protoset(protoset),
            metadata: vec![],
            body: r#"{"id":"123"}"#.into(),
        };

        let method = load_method(&request).unwrap();

        assert_eq!(method.name(), "GetUser");
        assert_eq!(method.input().full_name(), "example.GetUserRequest");
        assert_eq!(method.output().full_name(), "example.GetUserResponse");
    }

    #[test]
    fn message_template_includes_example_for_repeated_message_fields() {
        let tmp = Temp::new_dir().unwrap();
        let proto = tmp.join("order.proto");
        fs::write(
            &proto,
            r#"
                syntax = "proto3";
                package example;

                message CreateOrderRequest {
                    repeated LineItem items = 1;
                }

                message LineItem {
                    string sku = 1;
                    int32 quantity = 2;
                }
            "#,
        )
        .unwrap();
        let descriptors = protox::compile([proto.as_path()], [tmp.as_path()]).unwrap();
        let pool = DescriptorPool::from_file_descriptor_set(descriptors).unwrap();
        let request = pool
            .get_message_by_name("example.CreateOrderRequest")
            .unwrap();

        let template = message_template(&request).unwrap();

        assert_eq!(
            template,
            serde_json::json!({
                "items": [
                    {
                        "sku": "",
                        "quantity": 0,
                    }
                ]
            })
        );
    }
}
