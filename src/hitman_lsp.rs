use std::{
    collections::HashMap,
    fs,
    ops::Range as ByteRange,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use hitman::{
    env::{get_target, load_env},
    http_file,
    resolve::{find_root_dir, resolve_path, Resolved, ResolvedAs},
    scope::Replacement,
    transport::grpc::{
        list_methods, list_services, load_descriptor_pool, message_template,
        message_template_with_choices, next_message_template_choice, DescriptorSource,
        MessageTemplateChoiceGroup,
    },
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use toml::{Table as TomlTable, Value as TomlValue};
use toml_edit::{Document, Item as TomlEditItem, Table as TomlEditTable};
use tower_lsp::{
    async_trait,
    jsonrpc::Result as LspResult,
    lsp_types::{
        CodeAction, CodeActionKind, CodeActionOptions, CodeActionOrCommand, CodeActionParams,
        CodeActionProviderCapability, Command, CompletionItem, CompletionItemKind,
        CompletionOptions, CompletionParams, CompletionResponse, CompletionTextEdit, Diagnostic,
        DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
        DidOpenTextDocumentParams, ExecuteCommandOptions, ExecuteCommandParams,
        GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
        HoverProviderCapability, InitializeParams, InitializeResult, InsertTextFormat, Location,
        MarkedString, MessageType, OneOf, Position, Range, ReferenceParams, ServerCapabilities,
        TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit,
    },
    Client, LanguageServer, LspService, Server,
};

use prost_reflect::{Cardinality, FieldDescriptor, Kind, MessageDescriptor};

#[tokio::main]
async fn main() -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: Arc::new(Mutex::new(HashMap::new())),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
    Ok(())
}

const LIST_GRPC_METHODS: &str = "hitman.listGrpcMethods";
const NEXT_TEMPLATE_CHOICE: &str = "hitman.nextMessageTemplateChoice";
const SET_GRPC_METHOD: &str = "hitman.setGrpcMethod";
const SELECT_GRPC_METHOD: &str = "hitman.selectGrpcMethod";
const CONFIG_FILE: &str = "hitman.toml";
const LOCAL_CONFIG_FILE: &str = "hitman.local.toml";
const DATA_FILE: &str = ".hitman-data.toml";

#[derive(Clone)]
struct Backend {
    client: Client,
    documents: Arc<Mutex<HashMap<Url, String>>>,
}

impl Backend {
    async fn update_document(&self, uri: Url, text: String, version: i32) {
        // Serialize updates and publication so older edits cannot overwrite
        // diagnostics for newer document versions.
        let mut documents = self.documents.lock().await;
        documents.insert(uri.clone(), text.clone());
        let diagnostic_uri = uri.clone();
        let diagnostics =
            tokio::task::spawn_blocking(move || diagnostics_for_document(&diagnostic_uri, &text))
                .await
                .unwrap_or_default();
        self.client
            .publish_diagnostics(uri, diagnostics, Some(version))
            .await;
    }
}

#[async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> LspResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::REFACTOR]),
                        ..Default::default()
                    },
                )),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        LIST_GRPC_METHODS.to_string(),
                        NEXT_TEMPLATE_CHOICE.to_string(),
                        SET_GRPC_METHOD.to_string(),
                    ],
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["\"".to_string(), "/".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: tower_lsp::lsp_types::InitializedParams) {}

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = params.text_document;
        self.update_document(document.uri, document.text, document.version)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.update_document(
                params.text_document.uri,
                change.text,
                params.text_document.version,
            )
            .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut documents = self.documents.lock().await;
        let uri = params.text_document.uri;
        documents.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> LspResult<Option<Vec<CodeActionOrCommand>>> {
        let uri = params.text_document.uri;
        let Some(text) = self.documents.lock().await.get(&uri).cloned() else {
            return Ok(None);
        };

        match code_actions_for_document(&uri, &text) {
            Ok(actions) => Ok(Some(actions)),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let params = params.text_document_position_params;
        let uri = params.text_document.uri;
        let Some(text) = self.documents.lock().await.get(&uri).cloned() else {
            return Ok(None);
        };

        match hover_for_position(&uri, &text, params.position) {
            Ok(hover) => Ok(hover),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let params = params.text_document_position;
        let uri = params.text_document.uri;
        let Some(text) = self.documents.lock().await.get(&uri).cloned() else {
            return Ok(None);
        };

        match completion_for_position(&uri, &text, params.position) {
            Ok(completion) => Ok(completion),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        let params = params.text_document_position_params;
        let uri = params.text_document.uri;
        let Some(text) = self.documents.lock().await.get(&uri).cloned() else {
            return Ok(None);
        };

        match definition_for_position(&uri, &text, params.position) {
            Ok(definition) => Ok(definition.map(GotoDefinitionResponse::Array)),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn references(&self, params: ReferenceParams) -> LspResult<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let Some(text) = self.documents.lock().await.get(&uri).cloned() else {
            return Ok(None);
        };

        match references_for_position(
            &uri,
            &text,
            params.text_document_position.position,
            params.context.include_declaration,
        ) {
            Ok(references) => Ok(references),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> LspResult<Option<serde_json::Value>> {
        match params.command.as_str() {
            LIST_GRPC_METHODS => {
                let Ok(ListGrpcMethodsArgs { uri }) = parse_command_args(params.arguments) else {
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            "Invalid hitman.listGrpcMethods arguments",
                        )
                        .await;
                    return Ok(None);
                };

                let Some(text) = self.documents.lock().await.get(&uri).cloned() else {
                    return Ok(None);
                };

                match grpc_methods_for_document(&uri, &text) {
                    Ok(methods) => Ok(serde_json::to_value(methods).ok()),
                    Err(err) => {
                        self.client
                            .show_message(MessageType::ERROR, err.to_string())
                            .await;
                        Ok(None)
                    }
                }
            }
            NEXT_TEMPLATE_CHOICE => {
                let Ok(NextTemplateChoiceArgs {
                    uri,
                    method,
                    oneofs,
                }) = parse_command_args(params.arguments)
                else {
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            "Invalid hitman.nextMessageTemplateChoice arguments",
                        )
                        .await;
                    return Ok(None);
                };

                let Some(text) = self.documents.lock().await.get(&uri).cloned() else {
                    return Ok(None);
                };

                match template_choice_for_method(
                    &uri,
                    &text,
                    &method,
                    &oneof_selection_map(&oneofs),
                ) {
                    Ok(choice) => Ok(serde_json::to_value(choice).ok()),
                    Err(err) => {
                        self.client
                            .show_message(MessageType::ERROR, err.to_string())
                            .await;
                        Ok(None)
                    }
                }
            }
            SET_GRPC_METHOD => {
                let Ok(SetGrpcMethodArgs {
                    uri,
                    method,
                    oneofs,
                }) = parse_command_args(params.arguments)
                else {
                    self.client
                        .log_message(MessageType::ERROR, "Invalid hitman.setGrpcMethod arguments")
                        .await;
                    return Ok(None);
                };

                let Some(text) = self.documents.lock().await.get(&uri).cloned() else {
                    return Ok(None);
                };

                match set_grpc_method_edit(&uri, &text, &method, &oneofs) {
                    Ok(edit) => {
                        let applied = self
                            .client
                            .apply_edit(edit)
                            .await
                            .map(|response| response.applied)
                            .unwrap_or(false);
                        if !applied {
                            self.client
                                .show_message(
                                    MessageType::ERROR,
                                    "Editor rejected hitman method edit",
                                )
                                .await;
                        }
                    }
                    Err(err) => {
                        self.client
                            .show_message(MessageType::ERROR, err.to_string())
                            .await;
                    }
                }

                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct NextTemplateChoiceArgs {
    uri: Url,
    method: String,
    #[serde(default)]
    oneofs: Vec<GenerateTemplateOneofArgs>,
}

#[derive(Clone, Serialize, Deserialize)]
struct GenerateTemplateOneofArgs {
    oneof: String,
    field: String,
}

#[derive(Serialize, Deserialize)]
struct SetGrpcMethodArgs {
    uri: Url,
    method: String,
    #[serde(default)]
    oneofs: Vec<GenerateTemplateOneofArgs>,
}

#[derive(Serialize, Deserialize)]
struct ListGrpcMethodsArgs {
    uri: Url,
}

#[derive(Debug, PartialEq, Serialize)]
struct GrpcMethodItem {
    label: String,
    method: String,
}

fn parse_command_args<T: for<'de> Deserialize<'de>>(mut args: Vec<serde_json::Value>) -> Result<T> {
    let value = args.pop().context("Missing hitman command arguments")?;

    Ok(serde_json::from_value(value)?)
}

fn grpc_methods_for_document(uri: &Url, text: &str) -> Result<Vec<GrpcMethodItem>> {
    let path = uri_to_path(uri)?;
    let descriptor_source = descriptor_source(&path, text)?;
    let service = proto_service(&path, text, &descriptor_source)?;
    let current_method = grpc_method_name(text);
    let mut methods = list_methods(&descriptor_source, &service)?
        .into_iter()
        .map(|method| GrpcMethodItem {
            label: method.name().to_string(),
            method: method.full_name().to_string(),
        })
        .collect::<Vec<_>>();

    if let Some(current_method) = current_method {
        methods.sort_by_key(|method| method.label != current_method);
    }

    Ok(methods)
}

fn code_actions_for_document(uri: &Url, text: &str) -> Result<Vec<CodeActionOrCommand>> {
    let methods = grpc_methods_for_document(uri, text)?;
    if methods.is_empty() {
        return Ok(Vec::new());
    }

    let current_method = grpc_method_name(text);
    let mut actions = Vec::new();

    if let Some(current_method) = current_method {
        if let Some(method) = methods.iter().find(|method| method.label == current_method) {
            actions.push(generate_template_code_action(uri, &method.method));
        }
    }

    let title = "Select gRPC method".to_string();
    actions.push(CodeActionOrCommand::CodeAction(CodeAction {
        title: title.clone(),
        kind: Some(CodeActionKind::REFACTOR),
        command: Some(Command {
            title,
            command: SELECT_GRPC_METHOD.to_string(),
            arguments: Some(vec![serde_json::json!({
                "uri": uri,
                "methods": methods,
            })]),
        }),
        ..Default::default()
    }));

    Ok(actions)
}

fn generate_template_code_action(uri: &Url, method: &str) -> CodeActionOrCommand {
    let title = "Generate message template".to_string();
    CodeActionOrCommand::CodeAction(CodeAction {
        title: title.clone(),
        kind: Some(CodeActionKind::REFACTOR),
        command: Some(Command {
            title,
            command: SET_GRPC_METHOD.to_string(),
            arguments: Some(vec![serde_json::json!({
                "uri": uri,
                "method": method,
            })]),
        }),
        ..Default::default()
    })
}

fn template_choice_for_method(
    uri: &Url,
    text: &str,
    full_method: &str,
    choices: &HashMap<String, String>,
) -> Result<Option<MessageTemplateChoiceGroup>> {
    let path = uri_to_path(uri)?;
    let descriptor_source = descriptor_source(&path, text)?;
    let (service, method_name) = full_method
        .rsplit_once('.')
        .context("Invalid gRPC method")?;
    let method = list_methods(&descriptor_source, service)?
        .into_iter()
        .find(|method| method.name() == method_name)
        .with_context(|| format!("gRPC method not found: {full_method}"))?;

    Ok(next_message_template_choice(&method.input(), choices))
}

#[derive(Debug, Clone, PartialEq)]
struct VariableOccurrence {
    key: String,
    range: Range,
}

struct VariableDefinition {
    location: Location,
    value: Option<TomlValue>,
    array_entry: bool,
}

struct VariableResolution {
    occurrence: VariableOccurrence,
    declarations: Vec<VariableDefinition>,
    active: Option<usize>,
}

impl VariableResolution {
    fn active(&self) -> Option<&VariableDefinition> {
        self.active.and_then(|index| self.declarations.get(index))
    }
}

struct TomlKeyDefinition {
    range: Range,
    value: Option<TomlValue>,
    array_entry: bool,
}

fn hover_for_position(uri: &Url, text: &str, position: Position) -> Result<Option<Hover>> {
    let Some(resolution) = resolve_variable_at_position(uri, text, position)? else {
        return Ok(None);
    };
    let Some(value) = resolution
        .active()
        .and_then(|definition| definition.value.as_ref())
    else {
        return Ok(None);
    };

    Ok(Some(Hover {
        contents: HoverContents::Scalar(MarkedString::String(format!(
            "`{}`",
            toml_value_display(value)
        ))),
        range: Some(resolution.occurrence.range),
    }))
}

fn definition_for_position(
    uri: &Url,
    text: &str,
    position: Position,
) -> Result<Option<Vec<Location>>> {
    let Some(resolution) = resolve_variable_at_position(uri, text, position)? else {
        return Ok(None);
    };
    let Some(active_index) = resolution.active else {
        return Ok(None);
    };
    let active = &resolution.declarations[active_index];
    if !active.array_entry {
        return Ok(Some(vec![active.location.clone()]));
    }

    Ok(Some(
        resolution.declarations[active_index..]
            .iter()
            .take_while(|definition| {
                definition.array_entry && definition.location.uri == active.location.uri
            })
            .map(|definition| definition.location.clone())
            .collect(),
    ))
}

fn references_for_position(
    uri: &Url,
    text: &str,
    position: Position,
    include_declaration: bool,
) -> Result<Option<Vec<Location>>> {
    let Some(resolution) = resolve_variable_at_position(uri, text, position)? else {
        return Ok(None);
    };
    let path = uri_to_path(uri)?;
    let resolved = resolve_path(&path)?;
    let mut locations = Vec::new();

    for file_path in request_files(&resolved.root_dir)? {
        let Ok(file_text) = fs::read_to_string(&file_path) else {
            continue;
        };
        let file_uri =
            Url::from_file_path(&file_path).map_err(|_| anyhow::anyhow!("Invalid file path"))?;
        locations.extend(
            variable_occurrences(&file_text)
                .into_iter()
                .filter(|found| found.key == resolution.occurrence.key)
                .map(|found| Location {
                    uri: file_uri.clone(),
                    range: found.range,
                }),
        );
    }

    if include_declaration {
        locations.extend(
            resolution
                .declarations
                .into_iter()
                .map(|declaration| declaration.location),
        );
    }

    locations.sort_by(|a, b| {
        a.uri
            .as_str()
            .cmp(b.uri.as_str())
            .then(a.range.start.line.cmp(&b.range.start.line))
            .then(a.range.start.character.cmp(&b.range.start.character))
    });
    locations.dedup_by(|a, b| a.uri == b.uri && a.range == b.range);

    Ok(Some(locations))
}

#[derive(Debug, Default, PartialEq)]
struct JsonCompletionContext {
    path: Vec<String>,
    field: Option<String>,
    existing_fields: Vec<String>,
    key_start: Option<usize>,
    value_start: Option<usize>,
    expecting_key: bool,
    in_string: bool,
}

#[derive(Debug)]
struct JsonFrame {
    field: Option<String>,
    kind: JsonFrameKind,
}

#[derive(Debug, PartialEq)]
enum JsonFrameKind {
    Object,
    Array,
}

#[derive(PartialEq)]
enum JsonStringRole {
    Key,
    Value,
    Other,
}

struct JsonStringState {
    role: JsonStringRole,
    text: String,
    escaped: bool,
}

impl JsonStringState {
    /// Consume a character, returning true at the closing quote.
    fn push(&mut self, ch: char) -> bool {
        if self.escaped {
            self.text.push(ch);
            self.escaped = false;
        } else {
            match ch {
                '\\' => self.escaped = true,
                '"' => return true,
                _ => self.text.push(ch),
            }
        }
        false
    }
}

fn completion_for_position(
    uri: &Url,
    text: &str,
    position: Position,
) -> Result<Option<CompletionResponse>> {
    if let Some(completion) = grpc_method_completion(uri, text, position)? {
        return Ok(Some(completion));
    }
    let Some(body_start) = body_start_offset(text) else {
        return Ok(None);
    };
    let cursor = position_to_byte_offset(text, position);
    if cursor < body_start {
        return Ok(None);
    }

    let path = uri_to_path(uri)?;
    let descriptor_source = descriptor_source(&path, text)?;
    let service_name = proto_service(&path, text, &descriptor_source)?;
    let method_name = grpc_method_name(text).context("Missing gRPC method")?;
    let pool = load_descriptor_pool(&descriptor_source)?;
    let service = pool
        .get_service_by_name(&service_name)
        .with_context(|| format!("gRPC service not found: {service_name}"))?;
    let method = service
        .methods()
        .find(|method| method.name() == method_name)
        .with_context(|| format!("gRPC method not found: {method_name}"))?;
    let mut context = json_completion_context(&text[body_start..cursor]);
    context.existing_fields = existing_fields_for_path(&text[body_start..], &context.path);
    let Some(message) = message_for_path(method.input(), &context.path) else {
        return Ok(None);
    };

    let items = if context.expecting_key {
        field_completion_items(
            &message,
            &context.existing_fields,
            field_completion_range(text, body_start, cursor, context.key_start),
        )
    } else if let Some(field_name) = context.field.as_deref() {
        enum_completion_items(
            &message,
            field_name,
            context.in_string,
            enum_completion_edit(text, body_start, cursor, context.value_start),
        )
    } else {
        Vec::new()
    };

    if items.is_empty() {
        Ok(None)
    } else {
        Ok(Some(CompletionResponse::Array(items)))
    }
}

fn grpc_method_completion(
    uri: &Url,
    text: &str,
    position: Position,
) -> Result<Option<CompletionResponse>> {
    let Some(request) = grpc_request(text) else {
        return Ok(None);
    };
    let Some(target) = request.line.target else {
        return Ok(None);
    };
    let (line_index, line) = line_at_offset(text, target.range.start);
    if line_index != position.line as usize {
        return Ok(None);
    }
    let Some(slash) = target.text.rfind('/') else {
        return Ok(None);
    };
    let method = &target.text[slash + 1..];
    // A slash inside a template expression isn't a method separator.
    if method.contains(['{', '}', ' ']) {
        return Ok(None);
    }
    let line_start = text[..target.range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let method_start = target.range.start - line_start + slash + 1;
    let start = line[..method_start].encode_utf16().count() as u32;
    let end = start + method.encode_utf16().count() as u32;
    if position.character < start || position.character > end {
        return Ok(None);
    }

    let path = uri_to_path(uri)?;
    let base = diagnostic_template_value(&path, text, &target.text[..slash])?;
    let service_name = base.rsplit('/').next().unwrap_or_default();
    let source = descriptor_source(&path, text)?;
    let pool = load_descriptor_pool(&source)?;
    let Some(service) = pool.get_service_by_name(service_name) else {
        return Ok(None);
    };
    let range = Range::new(
        Position::new(position.line, start),
        Position::new(position.line, end),
    );
    let items = service
        .methods()
        .map(|method| CompletionItem {
            label: method.name().to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some(format!(
                "{} → {}",
                method.input().full_name(),
                method.output().full_name()
            )),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: method.name().to_string(),
            })),
            ..Default::default()
        })
        .collect();
    Ok(Some(CompletionResponse::Array(items)))
}

fn field_completion_items(
    message: &MessageDescriptor,
    existing_fields: &[String],
    replacement_range: Range,
) -> Vec<CompletionItem> {
    message
        .fields()
        .filter(|field| {
            !existing_fields
                .iter()
                .any(|existing| existing == field.json_name() || existing == field.name())
        })
        .map(|field| {
            let name = field.json_name().to_string();
            CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::FIELD),
                detail: Some(field_type_detail(&field)),
                filter_text: Some(format!("\"{name}\"")),
                insert_text: Some(field_completion_text(&field)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: replacement_range,
                    new_text: field_completion_text(&field),
                })),
                command: field_completion_command(&field),
                ..Default::default()
            }
        })
        .collect()
}

fn field_completion_range(
    text: &str,
    body_start: usize,
    cursor: usize,
    key_start: Option<usize>,
) -> Range {
    let start = body_start + key_start.unwrap_or(cursor - body_start);
    let end = if key_start.is_some() && text[cursor..].starts_with('"') {
        cursor + 1
    } else {
        cursor
    };
    Range {
        start: byte_position(text, start),
        end: byte_position(text, end),
    }
}

fn field_completion_text(field: &FieldDescriptor) -> String {
    let name = field.json_name();
    format!("\"{name}\": {}", field_value_snippet(field))
}

fn field_value_snippet(field: &FieldDescriptor) -> &'static str {
    if field.cardinality() == Cardinality::Repeated {
        return "[$0]";
    }
    if field.is_map() {
        return "{$0}";
    }

    match field.kind() {
        Kind::String | Kind::Enum(_) => "\"$0\"",
        Kind::Message(_) => "{$0}",
        _ => "$0",
    }
}

fn field_completion_command(field: &FieldDescriptor) -> Option<Command> {
    if matches!(singular_field_kind(field), Kind::Enum(_)) {
        Some(Command {
            title: "Suggest".to_string(),
            command: "editor.action.triggerSuggest".to_string(),
            arguments: None,
        })
    } else {
        None
    }
}

fn enum_completion_items(
    message: &MessageDescriptor,
    field_name: &str,
    in_string: bool,
    edit: Option<(Range, Option<TextEdit>)>,
) -> Vec<CompletionItem> {
    let Some(field) = field_by_json_name(message, field_name) else {
        return Vec::new();
    };
    let Kind::Enum(en) = singular_field_kind(&field) else {
        return Vec::new();
    };

    en.values()
        .map(|value| {
            let name = value.name().to_string();
            let mut item = CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(value.number().to_string()),
                insert_text: Some(if in_string {
                    format!("{name}$0")
                } else {
                    name.clone()
                }),
                insert_text_format: Some(if in_string {
                    InsertTextFormat::SNIPPET
                } else {
                    InsertTextFormat::PLAIN_TEXT
                }),
                ..Default::default()
            };
            if let Some((range, delete_closing_quote)) = edit.clone() {
                item.text_edit = Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: if delete_closing_quote.is_some() {
                        format!("{name}\"$0")
                    } else {
                        format!("{name}$0")
                    },
                }));
                item.additional_text_edits = delete_closing_quote.map(|edit| vec![edit]);
            }
            item
        })
        .collect()
}

fn enum_completion_edit(
    text: &str,
    body_start: usize,
    cursor: usize,
    value_start: Option<usize>,
) -> Option<(Range, Option<TextEdit>)> {
    let value_start = value_start?;
    let start = body_start + value_start;
    let delete_closing_quote = if text[cursor..].starts_with('"') {
        Some(TextEdit {
            range: Range {
                start: byte_position(text, cursor),
                end: byte_position(text, cursor + 1),
            },
            new_text: String::new(),
        })
    } else {
        None
    };

    Some((
        Range {
            start: byte_position(text, start),
            end: byte_position(text, cursor),
        },
        delete_closing_quote,
    ))
}

fn field_type_detail(field: &FieldDescriptor) -> String {
    if field.is_map() {
        return "object".to_string();
    }

    let ty = kind_type_detail(&field.kind());
    if field.cardinality() == Cardinality::Repeated {
        format!("array<{ty}>")
    } else {
        ty
    }
}

fn kind_type_detail(kind: &Kind) -> String {
    match kind {
        Kind::Double | Kind::Float => "number".to_string(),
        Kind::Int32
        | Kind::Sint32
        | Kind::Sfixed32
        | Kind::Int64
        | Kind::Sint64
        | Kind::Sfixed64
        | Kind::Uint32
        | Kind::Fixed32
        | Kind::Uint64
        | Kind::Fixed64 => "integer".to_string(),
        Kind::Bool => "boolean".to_string(),
        Kind::String => "string".to_string(),
        Kind::Bytes => "bytes".to_string(),
        Kind::Message(message) => message.full_name().to_string(),
        Kind::Enum(en) => en.full_name().to_string(),
    }
}

fn message_for_path(mut message: MessageDescriptor, path: &[String]) -> Option<MessageDescriptor> {
    for field_name in path {
        let field = field_by_json_name(&message, field_name)?;
        let Kind::Message(next) = singular_field_kind(&field) else {
            return None;
        };
        message = next;
    }

    Some(message)
}

fn field_by_json_name(message: &MessageDescriptor, field_name: &str) -> Option<FieldDescriptor> {
    message
        .fields()
        .find(|field| field.json_name() == field_name || field.name() == field_name)
}

fn singular_field_kind(field: &FieldDescriptor) -> Kind {
    if field.is_map() {
        return field.kind();
    }

    field.kind()
}

fn json_completion_context(prefix: &str) -> JsonCompletionContext {
    let mut frames: Vec<JsonFrame> = Vec::new();
    let mut pending_field: Option<String> = None;
    let mut last_string: Option<String> = None;
    let mut string: Option<JsonStringState> = None;
    let mut key_start = None;
    let mut value_start = None;
    let mut saw_colon_since_string = false;
    let mut last_non_ws = None;

    for (offset, ch) in prefix.char_indices() {
        if let Some(state) = string.as_mut() {
            if state.push(ch) {
                let state = string.take().unwrap();
                if state.role == JsonStringRole::Value {
                    pending_field = None;
                }
                last_string = Some(state.text);
                saw_colon_since_string = false;
                last_non_ws = Some('"');
            }
            continue;
        }

        if ch.is_whitespace() {
            continue;
        }

        match ch {
            '"' => {
                let role = if pending_field.is_some()
                    && matches!(last_non_ws, Some(':') | Some('[') | Some(','))
                {
                    value_start = Some(offset + 1);
                    JsonStringRole::Value
                } else if frames
                    .last()
                    .is_some_and(|frame| frame.kind == JsonFrameKind::Object)
                    && pending_field.is_none()
                    && matches!(last_non_ws, Some('{') | Some(','))
                {
                    key_start = Some(offset);
                    JsonStringRole::Key
                } else {
                    JsonStringRole::Other
                };
                string = Some(JsonStringState {
                    role,
                    text: String::new(),
                    escaped: false,
                });
            }
            ':' => {
                if let Some(key) = last_string.take() {
                    pending_field = Some(key);
                }
                saw_colon_since_string = true;
            }
            ',' | '{' | '[' | '}' | ']' => {
                match ch {
                    '{' => {
                        frames.push(JsonFrame {
                            field: pending_field.take().or_else(|| array_field(&frames)),
                            kind: JsonFrameKind::Object,
                        });
                    }
                    '[' => {
                        frames.push(JsonFrame {
                            field: pending_field.take(),
                            kind: JsonFrameKind::Array,
                        });
                    }
                    '}' | ']' => {
                        frames.pop();
                    }
                    _ => {}
                }
                pending_field = None;
                last_string = None;
                key_start = None;
                value_start = None;
                saw_colon_since_string = false;
            }
            _ => {}
        }
        last_non_ws = Some(ch);
    }

    let field = pending_field.or_else(|| array_field(&frames));
    let current_object = frames
        .last()
        .is_some_and(|frame| frame.kind == JsonFrameKind::Object);
    let expecting_key = current_object
        && field.is_none()
        && match &string {
            Some(state) => state.role == JsonStringRole::Key,
            None => matches!(last_non_ws, Some('{') | Some(',')),
        }
        && !saw_colon_since_string;

    JsonCompletionContext {
        path: object_path(&frames),
        field,
        key_start,
        value_start,
        expecting_key,
        in_string: string.is_some(),
        ..Default::default()
    }
}

fn array_field(frames: &[JsonFrame]) -> Option<String> {
    frames
        .last()
        .filter(|frame| frame.kind == JsonFrameKind::Array)
        .and_then(|frame| frame.field.clone())
}

fn existing_fields_for_path(body: &str, target_path: &[String]) -> Vec<String> {
    let mut frames: Vec<JsonFrame> = Vec::new();
    let mut last_string: Option<String> = None;
    let mut pending_field: Option<String> = None;
    let mut in_string = false;
    let mut escape = false;
    let mut string = String::new();
    let mut fields = Vec::new();

    for ch in body.chars() {
        if in_string {
            if escape {
                string.push(ch);
                escape = false;
                continue;
            }
            match ch {
                '\\' => escape = true,
                '"' => {
                    in_string = false;
                    last_string = Some(std::mem::take(&mut string));
                }
                _ => string.push(ch),
            }
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                string.clear();
            }
            ':' => {
                if let Some(key) = last_string.take() {
                    if object_path(&frames) == target_path {
                        fields.push(key.clone());
                    }
                    pending_field = Some(key);
                }
            }
            ',' => {
                last_string = None;
                pending_field = None;
            }
            '{' => {
                let field = pending_field.take().or_else(|| array_field(&frames));
                frames.push(JsonFrame {
                    field,
                    kind: JsonFrameKind::Object,
                });
                last_string = None;
            }
            '[' => {
                frames.push(JsonFrame {
                    field: pending_field.take(),
                    kind: JsonFrameKind::Array,
                });
                last_string = None;
            }
            '}' | ']' => {
                frames.pop();
                last_string = None;
                pending_field = None;
            }
            _ => {}
        }
    }

    fields.sort();
    fields.dedup();
    fields
}

fn object_path(frames: &[JsonFrame]) -> Vec<String> {
    frames
        .iter()
        .filter(|frame| frame.kind == JsonFrameKind::Object)
        .filter_map(|frame| frame.field.clone())
        .collect()
}

fn body_start_offset(text: &str) -> Option<usize> {
    text.find("\r\n\r\n")
        .map(|offset| offset + 4)
        .or_else(|| text.find("\n\n").map(|offset| offset + 2))
}

fn position_to_byte_offset(text: &str, position: Position) -> usize {
    let mut line = 0;
    let mut character = 0;

    for (offset, ch) in text.char_indices() {
        if line == position.line && character == position.character {
            return offset;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += 1;
        }
    }

    text.len()
}

fn resolve_variable_at_position(
    uri: &Url,
    text: &str,
    position: Position,
) -> Result<Option<VariableResolution>> {
    let Some(occurrence) = variable_at_position(text, position) else {
        return Ok(None);
    };
    let declarations = variable_definitions(uri, text, &occurrence.key)?;
    // Declarations are collected in runtime precedence order. Local declarations
    // come last, and the last local value wins when a name is declared twice.
    let active = declarations.len().checked_sub(1).map(|last| {
        if declarations[last].array_entry {
            // An array of tables is one value spread across several headers.
            // Navigate to its first header rather than treating the last entry
            // as the only effective definition.
            (0..=last)
                .rev()
                .take_while(|&index| {
                    declarations[index].array_entry
                        && declarations[index].location.uri == declarations[last].location.uri
                })
                .last()
                .unwrap_or(last)
        } else {
            last
        }
    });
    Ok(Some(VariableResolution {
        occurrence,
        declarations,
        active,
    }))
}

fn variable_definitions(
    request_uri: &Url,
    text: &str,
    key: &str,
) -> Result<Vec<VariableDefinition>> {
    let request_path = uri_to_path(request_uri)?;
    let resolved = resolve_path(&request_path)?;
    let target = get_target(&resolved.root_dir);
    let mut definitions = Vec::new();

    let config = resolved.root_dir.join(CONFIG_FILE);
    let local_config = resolved.root_dir.join(LOCAL_CONFIG_FILE);
    let request_config = resolved.toml_path();
    let data = resolved.root_dir.join(DATA_FILE);
    // Match load_env: merged global config, then the selected target, then
    // request config and data. Each later layer overrides earlier values.
    for (path, target) in [
        (&config, None),
        (&local_config, None),
        (&config, Some(target.as_str())),
        (&local_config, Some(target.as_str())),
        (&request_config, None),
        (&data, None),
    ] {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let uri = Url::from_file_path(path).map_err(|_| anyhow::anyhow!("Invalid file path"))?;
        definitions.extend(
            find_toml_key_definitions(&text, key, target)
                .into_iter()
                .map(|definition| VariableDefinition {
                    location: Location {
                        uri: uri.clone(),
                        range: definition.range,
                    },
                    value: definition.value,
                    array_entry: definition.array_entry,
                }),
        );
    }

    // Local variables are declared at the beginning of the .http file itself
    // (for example `@base_url = https://example.com`).
    let request_uri =
        Url::from_file_path(&request_path).map_err(|_| anyhow::anyhow!("Invalid file path"))?;
    definitions.extend(
        http_file::parse(text)
            .variables
            .into_iter()
            .filter(|variable| variable.name.text == key)
            .map(|variable| VariableDefinition {
                location: Location {
                    uri: request_uri.clone(),
                    range: byte_range(text, variable.name.range),
                },
                value: Some(TomlValue::String(variable.value.text.to_string())),
                array_entry: false,
            }),
    );

    Ok(definitions)
}

fn variable_at_position(text: &str, position: Position) -> Option<VariableOccurrence> {
    variable_occurrences(text)
        .into_iter()
        .find(|occurrence| position_in_range(position, occurrence.range))
        .or_else(|| {
            http_file::parse(text)
                .variables
                .into_iter()
                .find_map(|variable| {
                    let range = byte_range(text, variable.name.range);
                    position_in_range(position, range).then(|| VariableOccurrence {
                        key: variable.name.text.to_string(),
                        range,
                    })
                })
        })
}

fn position_in_range(position: Position, range: Range) -> bool {
    range.start.line == position.line
        && range.start.character <= position.character
        && position.character <= range.end.character
}

fn variable_occurrences(text: &str) -> Vec<VariableOccurrence> {
    let mut occurrences = Vec::new();

    for (line_index, line) in text.lines().enumerate() {
        let mut offset = 0;
        while let Some(start) = line[offset..].find("{{") {
            let start = offset + start;
            let Some(end) = line[start + 2..].find("}}") else {
                break;
            };
            let end = start + 2 + end;
            let key_start = start + 2;
            let key_end = end;
            let expression = &line[key_start..key_end];
            if let Some((key, key_offset)) = template_expression_key(expression) {
                let key_len = key.len();
                occurrences.push(VariableOccurrence {
                    key,
                    range: Range {
                        start: Position::new(line_index as u32, (key_start + key_offset) as u32),
                        end: Position::new(
                            line_index as u32,
                            (key_start + key_offset + key_len) as u32,
                        ),
                    },
                });
            }
            offset = end + 2;
        }
    }

    occurrences
}

fn template_expression_key(expression: &str) -> Option<(String, usize)> {
    let before_filter = expression.split('|').next()?;
    let key = before_filter.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return None;
    }

    Some((key.to_string(), before_filter.find(key).unwrap_or_default()))
}

fn find_toml_key_definitions(
    text: &str,
    key: &str,
    target: Option<&str>,
) -> Vec<TomlKeyDefinition> {
    let mut definitions = Vec::new();

    let Ok(document) = Document::parse(text) else {
        return definitions;
    };
    let root = document.as_table();

    if target.is_none() {
        push_toml_value_definition(text, root, key, &mut definitions);

        if let Some(oauth) = root.get("_oauth").and_then(TomlEditItem::as_table) {
            if let Some((profile_key, TomlEditItem::Table(_))) = oauth.get_key_value(key) {
                if let Some(range) = profile_key.span().map(|span| byte_range(text, span)) {
                    definitions.push(TomlKeyDefinition {
                        range,
                        value: None,
                        array_entry: false,
                    });
                }
            }
        }

        if let Some(TomlEditItem::ArrayOfTables(tables)) = root.get(key) {
            for table in tables.iter() {
                if let Some(range) = table
                    .span()
                    .and_then(|span| array_table_key_range(text, span, key))
                {
                    definitions.push(TomlKeyDefinition {
                        range,
                        value: None,
                        array_entry: true,
                    });
                }
            }
        }
    } else if let Some(table) = target
        .and_then(|target| root.get(target))
        .and_then(TomlEditItem::as_table)
    {
        push_toml_value_definition(text, table, key, &mut definitions);
    }

    definitions
}

fn push_toml_value_definition(
    text: &str,
    table: &TomlEditTable,
    key: &str,
    definitions: &mut Vec<TomlKeyDefinition>,
) {
    let Some((key, item)) = table.get_key_value(key) else {
        return;
    };
    let TomlEditItem::Value(value) = item else {
        return;
    };
    let Some(range) = key.span().map(|span| byte_range(text, span)) else {
        return;
    };

    definitions.push(TomlKeyDefinition {
        range,
        value: parse_toml_edit_value(value),
        array_entry: false,
    });
}

fn parse_toml_edit_value(value: &toml_edit::Value) -> Option<TomlValue> {
    toml::from_str::<TomlTable>(&format!("value = {value}"))
        .ok()?
        .remove("value")
}

fn byte_range(text: &str, range: ByteRange<usize>) -> Range {
    Range {
        start: byte_position(text, range.start),
        end: byte_position(text, range.end),
    }
}

fn array_table_key_range(text: &str, table_range: ByteRange<usize>, key: &str) -> Option<Range> {
    let header = text.get(table_range.clone())?.lines().next()?;
    let key_start = header.find(key)?;
    let start = table_range.start + key_start;

    Some(byte_range(text, start..start + key.len()))
}

fn byte_position(text: &str, offset: usize) -> Position {
    let mut line = 0;
    let mut line_start = 0;

    for (index, byte) in text.bytes().enumerate() {
        if index >= offset {
            break;
        }
        if byte == b'\n' {
            line += 1;
            line_start = index + 1;
        }
    }

    Position::new(line, (offset - line_start) as u32)
}

fn request_files(root_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root_dir) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        if is_request_file(entry.path()) {
            files.push(entry.path().to_path_buf());
        }
    }

    Ok(files)
}

fn is_request_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "http" | "gql" | "graphql"
            )
        })
        .unwrap_or(false)
}

fn toml_value_display(value: &TomlValue) -> String {
    match value {
        TomlValue::String(value) => value.clone(),
        TomlValue::Integer(value) => value.to_string(),
        TomlValue::Float(value) => value.to_string(),
        TomlValue::Boolean(value) => value.to_string(),
        TomlValue::Array(_) => value.to_string().replace('\n', " "),
        _ => value.to_string(),
    }
}

fn oneof_selection_map(oneofs: &[GenerateTemplateOneofArgs]) -> HashMap<String, String> {
    oneofs
        .iter()
        .map(|oneof| (oneof.oneof.clone(), oneof.field.clone()))
        .collect()
}

fn set_grpc_method_edit(
    uri: &Url,
    text: &str,
    full_method: &str,
    oneofs: &[GenerateTemplateOneofArgs],
) -> Result<WorkspaceEdit> {
    let path = uri_to_path(uri)?;
    let descriptor_source = descriptor_source(&path, text)?;
    let (service_name, method_name) = full_method
        .rsplit_once('.')
        .context("Invalid gRPC method")?;
    let method = list_methods(&descriptor_source, service_name)?
        .into_iter()
        .find(|method| method.name() == method_name)
        .with_context(|| format!("gRPC method not found: {full_method}"))?;
    let request = grpc_request(text).context("Missing GRPC request line")?;
    let (line_index, line) = line_at_offset(text, request.line.method.range.start);
    let target = request.line.target.context("Missing GRPC target")?;
    let line_start = text[..target.range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let new_line = format!(
        "{}{}{}",
        &line[..target.range.start - line_start],
        replace_grpc_target_method(target.text, service_name, method_name),
        &line[target.range.end - line_start..]
    );
    let method_edit = TextEdit {
        range: Range {
            start: Position::new(line_index as u32, 0),
            end: Position::new(line_index as u32, line.encode_utf16().count() as u32),
        },
        new_text: new_line,
    };
    let json = if oneofs.is_empty() {
        message_template(&method.input())?
    } else {
        message_template_with_choices(&method.input(), &oneof_selection_map(oneofs))?
    };
    let body_edit = TextEdit {
        range: replacement_range(text),
        new_text: replacement_text(text, &serde_json::to_string_pretty(&json)?),
    };
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![method_edit, body_edit]);

    Ok(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    })
}

fn replace_grpc_target_method(target: &str, service_name: &str, method_name: &str) -> String {
    let target = if let Some((base, current)) = target.rsplit_once('/') {
        if current.is_empty() || current.contains('}') || current == service_name {
            format!("{target}/{method_name}")
        } else {
            format!("{base}/{method_name}")
        }
    } else {
        format!("{target}/{method_name}")
    };

    target
}

fn diagnostic_for_line(line_index: usize, line: &str, message: String) -> Diagnostic {
    Diagnostic {
        range: Range::new(
            Position::new(line_index as u32, 0),
            Position::new(line_index as u32, line.encode_utf16().count() as u32),
        ),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("hitman".to_string()),
        message,
        ..Default::default()
    }
}

fn diagnostics_for_document(uri: &Url, text: &str) -> Vec<Diagnostic> {
    let Some(request) = grpc_request(text) else {
        return Vec::new();
    };
    let (request_index, request_line) = line_at_offset(text, request.line.method.range.start);
    let mut diagnostics = Vec::new();
    let path = match uri_to_path(uri) {
        Ok(path) => path,
        Err(_) => return diagnostics,
    };
    let target = diagnostic_template_value(
        &path,
        text,
        request
            .line
            .target
            .as_ref()
            .map_or("", |target| target.text),
    );
    if let Err(err) = &target {
        diagnostics.push(diagnostic_for_line(
            request_index,
            request_line,
            format!("{err:#}"),
        ));
    }

    let mut headers = request.headers.iter().filter(|header| {
        header.name.text.eq_ignore_ascii_case("proto")
            || header.name.text.eq_ignore_ascii_case("protoset")
    });
    let Some(header) = headers.next() else {
        diagnostics.push(diagnostic_for_line(
            request_index,
            request_line,
            "Missing Proto or Protoset header".to_string(),
        ));
        return diagnostics;
    };
    if let Some(duplicate) = headers.next() {
        let (index, line) = line_at_offset(text, duplicate.name.range.start);
        diagnostics.push(diagnostic_for_line(
            index,
            line,
            "Specify only one of Proto or Protoset".to_string(),
        ));
        return diagnostics;
    }
    let (index, line) = line_at_offset(text, header.name.range.start);
    let key = header.name.text;
    let value = header.value.text;

    let pool = (|| {
        let value = diagnostic_template_value(&path, text, value)?;
        anyhow::ensure!(!value.trim().is_empty(), "Missing {key} file path");
        let path = resolve_relative(&path, &value);
        let source = if key.eq_ignore_ascii_case("proto") {
            DescriptorSource::Proto(path)
        } else {
            DescriptorSource::Protoset(path)
        };
        load_descriptor_pool(&source).with_context(|| format!("Invalid {key} descriptors"))
    })();
    match pool {
        Err(err) => diagnostics.push(diagnostic_for_line(index, line, format!("{err:#}"))),
        Ok(pool) => {
            if let Ok(target) = target {
                if let Err(err) = validate_grpc_target(&pool, &target) {
                    diagnostics.push(diagnostic_for_line(
                        request_index,
                        request_line,
                        format!("{err:#}"),
                    ));
                }
            }
        }
    }
    diagnostics
}

// Render only static values: never invoke the request renderer, which can
// prompt, execute shell commands, or acquire OAuth tokens.
fn diagnostic_template_value(file_path: &Path, text: &str, value: &str) -> Result<String> {
    if !value.contains("{{") && !value.contains("{%") {
        return Ok(value.to_string());
    }
    let root_dir = find_root_dir(file_path)
        .unwrap_or_else(|| file_path.parent().unwrap_or(Path::new(".")).into());
    let resolved = Resolved {
        root_dir: root_dir.clone(),
        resolved_as: ResolvedAs::Simple {
            path: file_path.into(),
        },
    };
    let scope = load_env(&get_target(&root_dir), &resolved, &[])?;
    let mut environment = minijinja::Environment::new();
    environment.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    let mut rendered = value.to_string();
    let local_variables = http_file::parse(text).variables;
    for _ in 0..32 {
        if !rendered.contains("{{") && !rendered.contains("{%") {
            return Ok(rendered);
        }
        let mut values = HashMap::new();
        for occurrence in variable_occurrences(&rendered) {
            let local = local_variables
                .iter()
                .rev()
                .find(|variable| variable.name.text == occurrence.key);
            let replacement = if let Some(local) = local {
                local.value.text.to_string()
            } else {
                match scope.lookup(&occurrence.key)? {
                    Replacement::Value(value) => value,
                    Replacement::ValueNotFound { key } => {
                        anyhow::bail!("No value found for {{{{{key}}}}}")
                    }
                    Replacement::MultipleValuesFound { key, .. } => {
                        anyhow::bail!("Multiple values found for {{{{{key}}}}}")
                    }
                }
            };
            values.insert(occurrence.key, replacement);
        }
        rendered = environment.render_str(&rendered, values)?;
    }
    anyhow::bail!("Template expansion limit exceeded (possible cyclic variable reference)")
}

fn validate_grpc_target(pool: &prost_reflect::DescriptorPool, target: &str) -> Result<()> {
    let target = target.split_once("://").map_or(target, |(_, rest)| rest);
    let (host, path) = target
        .split_once('/')
        .context("Invalid gRPC target: expected host/package.Service/Method")?;
    anyhow::ensure!(!host.is_empty(), "Invalid gRPC target: missing host");
    let (service_name, method_name) = path
        .rsplit_once('/')
        .context("Invalid gRPC path: expected /package.Service/Method")?;
    let service = pool
        .get_service_by_name(service_name)
        .with_context(|| format!("gRPC service not found: {service_name}"))?;
    anyhow::ensure!(
        service.methods().any(|method| method.name() == method_name),
        "gRPC method not found: {service_name}/{method_name}"
    );
    Ok(())
}

fn descriptor_source(file_path: &Path, text: &str) -> Result<DescriptorSource> {
    let request = grpc_request(text).context("Missing GRPC request line")?;
    for header in request.headers {
        let key = header.name.text;
        let value = header.value.text;
        if key.eq_ignore_ascii_case("proto") {
            return Ok(DescriptorSource::Proto(resolve_relative(file_path, value)));
        }
        if key.eq_ignore_ascii_case("protoset") {
            return Ok(DescriptorSource::Protoset(resolve_relative(
                file_path, value,
            )));
        }
    }

    anyhow::bail!("Missing Proto or Protoset header")
}

fn proto_service(file_path: &Path, text: &str, source: &DescriptorSource) -> Result<String> {
    let target = grpc_target(text).context("Missing GRPC request line")?;
    let resolved_target = if let Some(var) = template_variable(&target) {
        resolve_hitman_variable(file_path, &var)?
    } else {
        target
    };
    let services = list_services(source)?;

    services
        .into_iter()
        .find(|service| resolved_target == *service || resolved_target.contains(service))
        .context("API not found in descriptors")
}

fn grpc_target(text: &str) -> Option<String> {
    grpc_request(text)?
        .line
        .target
        .map(|target| target.text.to_string())
}

fn grpc_request(text: &str) -> Option<http_file::Request<'_>> {
    http_file::parse(text)
        .request
        .filter(|request| request.line.method.text == "GRPC")
}

fn line_at_offset(text: &str, offset: usize) -> (usize, &str) {
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line_end = text[offset..]
        .find('\n')
        .map_or(text.len(), |index| offset + index);
    let line = text[line_start..line_end].trim_end_matches('\r');
    (
        text[..line_start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count(),
        line,
    )
}

fn grpc_method_name(text: &str) -> Option<String> {
    grpc_target(text)?.rsplit('/').next().map(str::to_string)
}

fn template_variable(target: &str) -> Option<String> {
    let start = target.find("{{")? + 2;
    let end = target[start..].find("}}")? + start;
    Some(target[start..end].trim().to_string())
}

fn resolve_hitman_variable(file_path: &Path, key: &str) -> Result<String> {
    let root = find_root_dir(file_path).context("Could not find hitman root")?;
    let resolved = Resolved {
        root_dir: root.clone(),
        resolved_as: ResolvedAs::Simple {
            path: file_path.into(),
        },
    };
    let target = get_target(&root);
    let scope = load_env(&target, &resolved, &[])?;

    match scope.lookup(key)? {
        Replacement::Value(value) => Ok(value),
        Replacement::ValueNotFound { key } => {
            anyhow::bail!("No value found for {{{{{key}}}}}")
        }
        Replacement::MultipleValuesFound { key, .. } => {
            anyhow::bail!("Multiple values found for {{{{{key}}}}}")
        }
    }
}

fn replacement_range(text: &str) -> Range {
    let lines: Vec<_> = text.lines().collect();
    let start_line = lines
        .iter()
        .position(|line| line.trim_start().starts_with('{'))
        .unwrap_or_else(|| {
            lines
                .iter()
                .rposition(|line| !line.trim().is_empty())
                .map(|line| line + 1)
                .unwrap_or(0)
        });

    Range {
        start: Position::new(start_line as u32, 0),
        end: Position::new(lines.len() as u32, 0),
    }
}

fn replacement_text(text: &str, body: &str) -> String {
    if text.lines().any(|line| line.trim_start().starts_with('{')) {
        body.to_string()
    } else {
        format!("\n{body}")
    }
}

fn resolve_relative(file_path: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        file_path.parent().unwrap_or(Path::new(".")).join(path)
    }
}

fn uri_to_path(uri: &Url) -> Result<PathBuf> {
    uri.to_file_path()
        .map_err(|_| anyhow::anyhow!("Only file:// URIs are supported"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mktemp::Temp;
    use prost::Message;
    use std::fs;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};

    struct LspTestClient {
        reader: BufReader<DuplexStream>,
        writer: DuplexStream,
        server: tokio::task::JoinHandle<()>,
        next_id: u64,
        applied_edit: Option<serde_json::Value>,
    }

    impl Drop for LspTestClient {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    impl LspTestClient {
        async fn start() -> Self {
            let (server_input, client_output) = tokio::io::duplex(64 * 1024);
            let (client_input, server_output) = tokio::io::duplex(64 * 1024);
            let (service, socket) = LspService::new(|client| Backend {
                client,
                documents: Arc::new(Mutex::new(HashMap::new())),
            });
            let server = tokio::spawn(async move {
                Server::new(server_input, server_output, socket)
                    .serve(service)
                    .await;
            });
            let mut client = Self {
                reader: BufReader::new(client_input),
                writer: client_output,
                server,
                next_id: 1,
                applied_edit: None,
            };
            client
                .request(
                    "initialize",
                    serde_json::json!({
                        "processId": null,
                        "rootUri": null,
                        "capabilities": {}
                    }),
                )
                .await;
            client.notify("initialized", serde_json::json!({})).await;
            client
        }

        async fn send(&mut self, message: serde_json::Value) {
            let body = serde_json::to_vec(&message).unwrap();
            self.writer
                .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
                .await
                .unwrap();
            self.writer.write_all(&body).await.unwrap();
        }

        async fn read(&mut self) -> serde_json::Value {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let mut length = None;
                loop {
                    let mut header = String::new();
                    assert!(self.reader.read_line(&mut header).await.unwrap() > 0);
                    if header == "\r\n" {
                        break;
                    }
                    if let Some(value) = header.strip_prefix("Content-Length: ") {
                        length = Some(value.trim().parse::<usize>().unwrap());
                    }
                }
                let mut body = vec![0; length.expect("Content-Length header")];
                self.reader.read_exact(&mut body).await.unwrap();
                serde_json::from_slice(&body).unwrap()
            })
            .await
            .expect("LSP response timed out")
        }

        async fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
            let id = self.next_id;
            self.next_id += 1;
            self.send(serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": params
            }))
            .await;
            loop {
                let message = self.read().await;
                if message["method"] == "workspace/applyEdit" {
                    self.applied_edit = Some(message["params"]["edit"].clone());
                    self.send(serde_json::json!({
                        "jsonrpc": "2.0", "id": message["id"], "result": {"applied": true}
                    }))
                    .await;
                    continue;
                }
                if message.get("id") == Some(&serde_json::json!(id)) {
                    assert!(message.get("error").is_none(), "{message}");
                    return message["result"].clone();
                }
            }
        }

        async fn notify(&mut self, method: &str, params: serde_json::Value) {
            self.send(serde_json::json!({
                "jsonrpc": "2.0", "method": method, "params": params
            }))
            .await;
        }

        async fn open(&mut self, uri: &Url, text: &str) -> serde_json::Value {
            self.notify(
                "textDocument/didOpen",
                serde_json::json!({
                    "textDocument": {"uri": uri, "languageId": "http", "version": 1, "text": text}
                }),
            )
            .await;
            // didOpen has no response; diagnostics confirm the document is ready.
            loop {
                let message = self.read().await;
                if message["method"] == "textDocument/publishDiagnostics"
                    && message["params"]["uri"] == serde_json::json!(uri)
                {
                    return message["params"].clone();
                }
            }
        }

        async fn change(&mut self, uri: &Url, text: &str) -> serde_json::Value {
            self.notify(
                "textDocument/didChange",
                serde_json::json!({
                    "textDocument": {"uri": uri, "version": 2},
                    "contentChanges": [{"text": text}]
                }),
            )
            .await;
            loop {
                let message = self.read().await;
                if message["method"] == "textDocument/publishDiagnostics"
                    && message["params"]["uri"] == serde_json::json!(uri)
                    && message["params"]["version"] == 2
                {
                    return message["params"].clone();
                }
            }
        }

        async fn at(
            &mut self,
            method: &str,
            uri: &Url,
            line: u32,
            character: u32,
        ) -> serde_json::Value {
            self.request(
                method,
                serde_json::json!({
                "textDocument": {"uri": uri},
                "position": {"line": line, "character": character},
                "context": {"includeDeclaration": true, "triggerKind": 1}
                }),
            )
            .await
        }

        async fn references(
            &mut self,
            uri: &Url,
            line: u32,
            character: u32,
            include_declaration: bool,
        ) -> serde_json::Value {
            self.request(
                "textDocument/references",
                serde_json::json!({
                    "textDocument": {"uri": uri},
                    "position": {"line": line, "character": character},
                    "context": {"includeDeclaration": include_declaration}
                }),
            )
            .await
        }
    }

    fn grpc_fixture() -> (Temp, PathBuf, String) {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("add.proto"),
            r#"
                syntax = "proto3";
                package addsvc;

                service Add {
                    rpc Sum (SumRequest) returns (SumResponse);
                    rpc Concat (ConcatRequest) returns (ConcatResponse);
                }

                message SumRequest {
                    int32 a = 1;
                    int32 b = 2;
                }

                message SumResponse { int32 result = 1; }
                message ConcatRequest { string a = 1; string b = 2; }
                message ConcatResponse { string result = 1; }
            "#,
        )
        .unwrap();
        let descriptors = protox::compile([tmp.join("add.proto")], [tmp.as_path()]).unwrap();
        fs::write(tmp.join("protoset.bin"), descriptors.encode_to_vec()).unwrap();

        let request = tmp.join("sum.http");
        let text = r#"
GRPC grpcb.in:9001/addsvc.Add/Sum
Protoset: ./protoset.bin

{}
"#
        .strip_prefix('\n')
        .unwrap()
        .to_string();
        fs::write(&request, &text).unwrap();

        (tmp, request, text)
    }

    fn grpc_completion_fixture(body: &str) -> (Temp, PathBuf, String) {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("orders.proto"),
            r#"
                syntax = "proto3";
                package orders;

                service Orders {
                    rpc Create (CreateOrderRequest) returns (CreateOrderResponse);
                }

                enum State {
                    STATE_UNSPECIFIED = 0;
                    ACTIVE = 1;
                    DISABLED = 2;
                }

                message CreateOrderRequest {
                    string user_id = 1;
                    State state = 2;
                    Nested nested = 3;
                    repeated State states = 4;
                }

                message Nested {
                    State state = 1;
                    string note = 2;
                }

                message CreateOrderResponse { string id = 1; }
            "#,
        )
        .unwrap();
        let descriptors = protox::compile([tmp.join("orders.proto")], [tmp.as_path()]).unwrap();
        fs::write(tmp.join("protoset.bin"), descriptors.encode_to_vec()).unwrap();

        let request = tmp.join("create.http");
        let text = format!(
            r#"
GRPC localhost:50051/orders.Orders/Create
Protoset: ./protoset.bin

{body}"#
        )
        .strip_prefix('\n')
        .unwrap()
        .to_string();
        fs::write(&request, &text).unwrap();

        (tmp, request, text)
    }

    fn grpc_oneof_fixture() -> (Temp, PathBuf, String) {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("petstore.proto"),
            r#"
                syntax = "proto3";
                package petstore;

                service Pets {
                    rpc Adopt (AdoptRequest) returns (AdoptResponse);
                }

                message AdoptRequest {
                    oneof pet {
                        Cat cat = 1;
                        Dog dog = 2;
                    }
                }

                message Cat {
                    string name = 1;
                    oneof toy {
                        Ball ball = 2;
                        Mouse mouse = 3;
                    }
                }

                message Ball { string color = 1; }
                message Mouse { string material = 1; }
                message Dog { string name = 1; }
                message AdoptResponse { string id = 1; }
            "#,
        )
        .unwrap();
        let descriptors = protox::compile([tmp.join("petstore.proto")], [tmp.as_path()]).unwrap();
        fs::write(tmp.join("protoset.bin"), descriptors.encode_to_vec()).unwrap();

        let request = tmp.join("adopt.http");
        let text = r#"
GRPC localhost:50051/petstore.Pets/Adopt
Protoset: ./protoset.bin

{}
"#
        .strip_prefix('\n')
        .unwrap()
        .to_string();
        fs::write(&request, &text).unwrap();

        (tmp, request, text)
    }

    async fn open_request(tmp: &Temp, text: &str) -> (Url, LspTestClient) {
        let request = tmp.join("request.http");
        fs::write(&request, text).unwrap();
        let uri = Url::from_file_path(request).unwrap();
        let mut client = LspTestClient::start().await;
        client.open(&uri, text).await;
        (uri, client)
    }

    #[tokio::test]
    async fn lsp_hover_shows_effective_config_value() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(tmp.join("hitman.toml"), r#"url = "global""#).unwrap();
        fs::write(tmp.join("hitman.local.toml"), r#"url = "local""#).unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{url}}").await;

        let hover = client.at("textDocument/hover", &uri, 0, 7).await;
        assert_eq!(hover["contents"], "`local`");
    }

    #[tokio::test]
    async fn lsp_definition_navigates_to_effective_config_declaration() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(tmp.join("hitman.toml"), r#"url = "global""#).unwrap();
        fs::write(tmp.join("hitman.local.toml"), r#"url = "local""#).unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{url}}").await;

        let definition = client.at("textDocument/definition", &uri, 0, 7).await;
        assert_eq!(definition.as_array().unwrap().len(), 1);
        assert_eq!(
            definition[0]["uri"],
            serde_json::json!(Url::from_file_path(tmp.join("hitman.local.toml")).unwrap())
        );
        assert_eq!(
            definition[0]["range"]["start"],
            serde_json::json!({"line": 0, "character": 0})
        );
    }

    #[tokio::test]
    async fn lsp_references_include_all_config_declarations() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(tmp.join("hitman.toml"), r#"url = "global""#).unwrap();
        fs::write(tmp.join("hitman.local.toml"), r#"url = "local""#).unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{url}}").await;

        let references = client.references(&uri, 0, 7, true).await;
        for file in ["hitman.toml", "hitman.local.toml"] {
            assert!(references.as_array().unwrap().iter().any(|reference| {
                reference["uri"] == serde_json::json!(Url::from_file_path(tmp.join(file)).unwrap())
                    && reference["range"]["start"] == serde_json::json!({"line": 0, "character": 0})
            }));
        }
    }

    #[tokio::test]
    async fn lsp_references_without_declarations_include_graphql_uses() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(tmp.join("hitman.toml"), r#"url = "global""#).unwrap();
        fs::write(
            tmp.join("query.graphql"),
            r#"query { field(arg: "{{url}}") }"#,
        )
        .unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{url}}").await;

        let uses = client.references(&uri, 0, 7, false).await;
        let uses = uses.as_array().unwrap();

        assert!(uses
            .iter()
            .any(|reference| { reference["uri"] == serde_json::json!(uri) }));
        assert!(uses.iter().any(|reference| reference["uri"]
            == serde_json::json!(Url::from_file_path(tmp.join("query.graphql")).unwrap())));
        assert!(uses.iter().all(|reference| reference["uri"]
            != serde_json::json!(Url::from_file_path(tmp.join("hitman.toml")).unwrap())));
    }

    #[tokio::test]
    async fn lsp_definition_prefers_data_over_config() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(tmp.join("hitman.toml"), "token = 'config'").unwrap();
        fs::write(tmp.join(".hitman-data.toml"), "token = 'data'").unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{token}}").await;

        let definition = client.at("textDocument/definition", &uri, 0, 8).await;
        assert_eq!(
            definition[0]["uri"],
            serde_json::json!(Url::from_file_path(tmp.join(".hitman-data.toml")).unwrap())
        );
    }

    #[tokio::test]
    async fn lsp_hover_shows_selected_target_only_value() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("hitman.toml"),
            r#"[dev]
target_only = "dev-only"
"#,
        )
        .unwrap();
        fs::write(tmp.join(".hitman-target"), "dev").unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{target_only}}").await;

        let hover = client.at("textDocument/hover", &uri, 0, 8).await;
        assert_eq!(hover["contents"], "`dev-only`");
    }

    async fn open_unsaved_local_fixture() -> (Temp, Url, LspTestClient) {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("hitman.toml"),
            r#"base_url = 'config'
"#,
        )
        .unwrap();
        let request = tmp.join("request.http");
        fs::write(
            &request,
            r#"GET {{base_url}}
"#,
        )
        .unwrap();
        let uri = Url::from_file_path(request).unwrap();
        let mut client = LspTestClient::start().await;
        let text = r#"@base_url = local # note

GET {{base_url}}
"#;
        client.open(&uri, text).await;
        (tmp, uri, client)
    }

    #[tokio::test]
    async fn lsp_hover_reads_unsaved_local_value() {
        let (_tmp, uri, mut client) = open_unsaved_local_fixture().await;
        assert_eq!(
            client.at("textDocument/hover", &uri, 2, 8).await["contents"],
            "`local`"
        );
    }

    #[tokio::test]
    async fn lsp_definition_navigates_to_unsaved_local_declaration() {
        let (_tmp, uri, mut client) = open_unsaved_local_fixture().await;
        assert_eq!(
            client.at("textDocument/definition", &uri, 2, 8).await[0]["range"]["start"],
            serde_json::json!({"line": 0, "character": 1})
        );
    }

    #[tokio::test]
    async fn lsp_hover_works_on_local_declaration() {
        let (_tmp, uri, mut client) = open_unsaved_local_fixture().await;
        assert_eq!(
            client.at("textDocument/hover", &uri, 0, 3).await["contents"],
            "`local`"
        );
    }

    #[tokio::test]
    async fn lsp_references_include_overridden_config_declaration() {
        let (tmp, uri, mut client) = open_unsaved_local_fixture().await;
        let refs = client.references(&uri, 2, 8, true).await;
        assert!(refs
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| reference["uri"]
                == serde_json::json!(Url::from_file_path(tmp.join("hitman.toml")).unwrap())));
    }

    #[tokio::test]
    async fn lsp_hover_updates_after_document_change() {
        let (_tmp, uri, mut client) = open_unsaved_local_fixture().await;
        client
            .change(
                &uri,
                r#"@base_url = updated

GET {{base_url}}
"#,
            )
            .await;
        assert_eq!(
            client.at("textDocument/hover", &uri, 2, 8).await["contents"],
            "`updated`"
        );
    }

    #[tokio::test]
    async fn lsp_hover_is_null_outside_a_variable() {
        let (_tmp, uri, mut client) = open_unsaved_local_fixture().await;
        assert_eq!(
            client.at("textDocument/hover", &uri, 2, 0).await,
            serde_json::Value::Null
        );
    }

    #[tokio::test]
    async fn lsp_hover_uses_last_local_declaration() {
        let tmp = Temp::new_dir().unwrap();
        let text = r#"
@id = old
@id = new

GET /{{id}}
"#;
        let (uri, mut client) = open_request(&tmp, text).await;

        assert_eq!(
            client.at("textDocument/hover", &uri, 4, 8).await["contents"],
            "`new`"
        );
    }

    #[tokio::test]
    async fn lsp_definition_uses_last_local_declaration() {
        let tmp = Temp::new_dir().unwrap();
        let text = r#"
@id = old
@id = new

GET /{{id}}
"#;
        let (uri, mut client) = open_request(&tmp, text).await;

        let definition = client.at("textDocument/definition", &uri, 4, 8).await;
        assert_eq!(
            definition[0]["range"]["start"],
            serde_json::json!({"line": 2, "character": 1})
        );
    }

    #[tokio::test]
    async fn lsp_references_include_both_local_declarations_but_not_comments() {
        let tmp = Temp::new_dir().unwrap();
        let text = r#"
# before
@  id = old # note
# @id = ignored
@id = new

GET /{{id}}
"#
        .replace('\n', "\r\n");
        let (uri, mut client) = open_request(&tmp, &text).await;

        let references = client.references(&uri, 6, 8, true).await;
        for line in [2, 4] {
            assert!(references
                .as_array()
                .unwrap()
                .iter()
                .any(|reference| { reference["range"]["start"]["line"] == line }));
        }
        assert!(!references
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| { reference["range"]["start"]["line"] == 3 }));
    }

    #[tokio::test]
    async fn lsp_hover_prefers_target_over_local_config_global() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("hitman.toml"),
            r#"
[dev]
url = 'target'
"#,
        )
        .unwrap();
        fs::write(tmp.join("hitman.local.toml"), "url = 'global-local'").unwrap();
        fs::write(tmp.join(".hitman-target"), "dev").unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{url}}").await;

        assert_eq!(
            client.at("textDocument/hover", &uri, 0, 7).await["contents"],
            "`target`"
        );
    }

    #[tokio::test]
    async fn lsp_definition_prefers_target_over_local_config_global() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("hitman.toml"),
            r#"
[dev]
url = 'target'
"#,
        )
        .unwrap();
        fs::write(tmp.join("hitman.local.toml"), "url = 'global-local'").unwrap();
        fs::write(tmp.join(".hitman-target"), "dev").unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{url}}").await;

        let definition = client.at("textDocument/definition", &uri, 0, 7).await;
        assert_eq!(
            definition[0]["uri"],
            serde_json::json!(Url::from_file_path(tmp.join("hitman.toml")).unwrap())
        );
        assert_eq!(
            definition[0]["range"]["start"],
            serde_json::json!({"line": 2, "character": 0})
        );
    }

    #[tokio::test]
    async fn lsp_references_include_overridden_global_config() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("hitman.toml"),
            r#"
[dev]
url = 'target'
"#,
        )
        .unwrap();
        fs::write(tmp.join("hitman.local.toml"), "url = 'global-local'").unwrap();
        fs::write(tmp.join(".hitman-target"), "dev").unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{url}}").await;

        let refs = client.references(&uri, 0, 7, true).await;
        assert!(refs
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| reference["uri"]
                == serde_json::json!(Url::from_file_path(tmp.join("hitman.local.toml")).unwrap())));
    }

    #[tokio::test]
    async fn lsp_definition_navigates_to_all_effective_array_table_headers() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("hitman.toml"),
            r#"
[[label]]
value = "bug"

[[label]]
value = "documentation"
"#,
        )
        .unwrap();
        let (uri, mut client) = open_request(
            &tmp,
            r#""labels": ["{{ label | select_multiple | join('", "') }}"]"#,
        )
        .await;

        let definitions = client.at("textDocument/definition", &uri, 0, 17).await;
        let definitions = definitions.as_array().unwrap();
        assert_eq!(definitions.len(), 2);
        let config_uri = Url::from_file_path(tmp.join("hitman.toml")).unwrap();
        for (definition, line) in definitions.iter().zip([1, 4]) {
            assert_eq!(definition["uri"], serde_json::json!(config_uri));
            assert_eq!(
                definition["range"]["start"],
                serde_json::json!({"line": line, "character": 2})
            );
        }
    }

    #[tokio::test]
    async fn lsp_definition_uses_only_effective_array_table_headers() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("hitman.toml"),
            r#"[[label]]
value = 'global'
"#,
        )
        .unwrap();
        fs::write(
            tmp.join("hitman.local.toml"),
            r#"[[label]]
value = 'local-a'

[[label]]
value = 'local-b'
"#,
        )
        .unwrap();
        let (uri, mut client) = open_request(&tmp, "GET {{label}}").await;

        let definitions = client.at("textDocument/definition", &uri, 0, 7).await;
        let definitions = definitions.as_array().unwrap();
        assert_eq!(definitions.len(), 2);
        let local_uri = Url::from_file_path(tmp.join("hitman.local.toml")).unwrap();
        for (definition, line) in definitions.iter().zip([0, 3]) {
            assert_eq!(definition["uri"], serde_json::json!(local_uri));
            assert_eq!(
                definition["range"]["start"],
                serde_json::json!({"line": line, "character": 2})
            );
        }
    }

    #[tokio::test]
    async fn lsp_references_include_all_array_table_headers() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("hitman.toml"),
            r#"
[[label]]
value = "bug"

[[label]]
value = "documentation"
"#,
        )
        .unwrap();
        let (uri, mut client) = open_request(
            &tmp,
            r#""labels": ["{{ label | select_multiple | join('", "') }}"]"#,
        )
        .await;

        let refs = client.references(&uri, 0, 17, true).await;
        for line in [1, 4] {
            assert!(refs
                .as_array()
                .unwrap()
                .iter()
                .any(|reference| reference["range"]["start"]
                    == serde_json::json!({"line": line, "character": 2})));
        }
    }

    #[tokio::test]
    async fn lsp_definition_finds_oauth_profile() {
        let tmp = Temp::new_dir().unwrap();
        fs::write(
            tmp.join("hitman.toml"),
            r#"[_oauth.github_access_token]
flow = 'client_credentials'
"#,
        )
        .unwrap();
        let (uri, mut client) = open_request(
            &tmp,
            r#"GET /
Authorization: Bearer {{github_access_token}}
"#,
        )
        .await;
        let definition = client.at("textDocument/definition", &uri, 1, 30).await;
        assert_eq!(
            definition[0]["range"]["start"],
            serde_json::json!({"line": 0, "character": 8})
        );
    }

    #[tokio::test]
    async fn lsp_reports_diagnostics_and_clears_them_after_edit() {
        let (_tmp, request, text) = grpc_fixture();
        let uri = Url::from_file_path(request).unwrap();
        let mut client = LspTestClient::start().await;
        let invalid = text.replace("/Sum", "/Missing");
        let diagnostics = client.open(&uri, &invalid).await;
        assert!(diagnostics["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| { diagnostic["message"].as_str().unwrap().contains("Missing") }));

        let diagnostics = client.change(&uri, &text).await;
        assert_eq!(diagnostics["diagnostics"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn lsp_accepts_a_request_with_a_leading_blank_line() {
        let (_tmp, request, _) = grpc_fixture();
        let uri = Url::from_file_path(request).unwrap();
        let text = r#"
GRPC grpcb.in:9001/addsvc.Add/Sum
Protoset: ./protoset.bin

{}
"#;
        let mut client = LspTestClient::start().await;
        let diagnostics = client.open(&uri, text).await;
        assert_eq!(diagnostics["diagnostics"], serde_json::json!([]));

        let actions = client
            .request(
                "textDocument/codeAction",
                serde_json::json!({
                    "textDocument": {"uri": uri},
                    "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 0}},
                    "context": {"diagnostics": []}
                }),
            )
            .await;
        assert_eq!(actions[0]["title"], "Generate message template");
    }

    #[tokio::test]
    async fn lsp_offers_grpc_code_actions() {
        let (tmp, _request, text) = grpc_fixture();
        let (uri, mut client) = open_request(&tmp, &text).await;

        let actions = client.request("textDocument/codeAction", serde_json::json!({
            "textDocument": {"uri": uri},
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
            "context": {"diagnostics": []}
        })).await;
        assert_eq!(actions[0]["title"], "Generate message template");
        assert_eq!(actions[1]["title"], "Select gRPC method");
    }

    #[tokio::test]
    async fn lsp_lists_grpc_methods() {
        let (tmp, _request, text) = grpc_fixture();
        let (uri, mut client) = open_request(&tmp, &text).await;

        let methods = client
            .request(
                "workspace/executeCommand",
                serde_json::json!({
                    "command": LIST_GRPC_METHODS, "arguments": [{"uri": uri}]
                }),
            )
            .await;
        assert_eq!(methods.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn lsp_completes_grpc_method_names() {
        let (tmp, _request, _) = grpc_fixture();
        let partial = r#"
GRPC grpcb.in:9001/addsvc.Add/
Protoset: ./protoset.bin
"#;
        let (uri, mut client) = open_request(&tmp, partial).await;

        let items = client
            .at(
                "textDocument/completion",
                &uri,
                1,
                partial.lines().nth(1).unwrap().len() as u32,
            )
            .await;
        assert!(items
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "Sum"));
        assert!(items
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "Concat"));
    }

    #[tokio::test]
    async fn lsp_completes_grpc_body_fields() {
        let (_tmp, request, text) = grpc_completion_fixture(
            r#"{

}
"#,
        );
        let uri = Url::from_file_path(request).unwrap();
        let mut client = LspTestClient::start().await;
        client.open(&uri, &text).await;

        let fields = client.at("textDocument/completion", &uri, 4, 0).await;
        for field in ["userId", "state", "nested", "states"] {
            assert!(fields
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["label"] == field));
        }
    }

    #[tokio::test]
    async fn lsp_completes_grpc_enum_values() {
        let (_tmp, request, text) = grpc_completion_fixture(
            r#"{
  "state": ""
}
"#,
        );
        let uri = Url::from_file_path(request).unwrap();
        let mut client = LspTestClient::start().await;
        client.open(&uri, &text).await;
        let values = client.at("textDocument/completion", &uri, 4, 12).await;
        assert!(values
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "ACTIVE"));
    }

    #[tokio::test]
    async fn lsp_lists_oneof_template_choices() {
        let (tmp, _request, text) = grpc_oneof_fixture();
        let (uri, mut client) = open_request(&tmp, &text).await;

        let choice = client
            .request(
                "workspace/executeCommand",
                serde_json::json!({
                    "command": NEXT_TEMPLATE_CHOICE,
                    "arguments": [{"uri": uri, "method": "petstore.Pets.Adopt"}]
                }),
            )
            .await;
        assert_eq!(choice["oneof"], "petstore.AdoptRequest.pet");
        assert_eq!(choice["options"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn lsp_lists_nested_oneof_template_choices() {
        let (tmp, _request, text) = grpc_oneof_fixture();
        let (uri, mut client) = open_request(&tmp, &text).await;

        let nested = client
            .request(
                "workspace/executeCommand",
                serde_json::json!({
                    "command": NEXT_TEMPLATE_CHOICE,
                    "arguments": [{"uri": uri, "method": "petstore.Pets.Adopt", "oneofs": [
                        {"oneof": "petstore.AdoptRequest.pet", "field": "cat"}
                    ]}]
                }),
            )
            .await;
        assert_eq!(nested["oneof"], "petstore.Cat.toy");
    }

    #[tokio::test]
    async fn lsp_applies_grpc_message_template_edit() {
        let (_tmp, request, text) = grpc_fixture();
        let uri = Url::from_file_path(request).unwrap();
        let mut client = LspTestClient::start().await;
        client.open(&uri, &text).await;

        client
            .request(
                "workspace/executeCommand",
                serde_json::json!({
                    "command": SET_GRPC_METHOD,
                    "arguments": [{"uri": uri, "method": "addsvc.Add.Sum"}]
                }),
            )
            .await;
        let edits = &client.applied_edit.as_ref().unwrap()["changes"][uri.as_str()];
        assert!(edits.as_array().unwrap().iter().any(|edit| {
            edit["newText"]
                == r#"{
  "a": 0,
  "b": 0
}"#
        }));
    }
}
