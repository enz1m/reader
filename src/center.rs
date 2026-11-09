//! Optional Center library and navigation commands using opaque book identifiers.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use enzui_iced_agent::center::{
    AppFunction, AppFunctionRegistry, AppMetadata, CenterFailure, CenterHost, CenterOutcome,
    CenterProvision, CenterTurn, ConnectorEvent, ConsentScope, FunctionRisk, FunctionSpec,
};
use iced::{Subscription, Task};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::App;
use crate::message::Msg;
use crate::model::Screen;

/// The app id the center credential is enrolled under.
pub const CENTER_APP_ID: &str = "io.github.enzui.Reader";
/// The most books one listing returns.
pub const LIST_LIMIT: usize = 200;

/// What an admitted call asks Reader to do.
#[derive(Debug, PartialEq, Eq)]
pub enum CenterCall {
    ListLibrary,
    OpenBook { book_id: String },
    TurnPage { forward: bool },
    ReadingStatus,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BookArgs {
    book_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnArgs {
    direction: Direction,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Next,
    Previous,
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties,
    })
}

fn status_schema() -> Value {
    object(
        json!({
            "reading": {"type": "boolean"},
            "title": {"type": "string"},
            "page": {"type": "integer"},
            "page_count": {"type": "integer"}
        }),
        &["reading", "title", "page", "page_count"],
    )
}

/// Reader's registered functions.
#[must_use]
pub fn registry() -> AppFunctionRegistry<CenterCall> {
    let once_or_session = vec![ConsentScope::Once, ConsentScope::Session];
    let list = FunctionSpec::new(
        "list_library",
        "List the library",
        "Lists known books, most recently opened first: id, title, author, format and \
         progress.",
        object(json!({}), &[]),
        object(
            json!({"books": {"type": "array", "items": object(json!({
                "book_id": {"type": "string"},
                "title": {"type": "string"},
                "author": {"type": "string"},
                "format": {"type": "string"},
                "progress_percentage": {"type": "number"},
                "available": {"type": "boolean"}
            }), &["book_id", "title", "author", "format", "progress_percentage", "available"])}}),
            &["books"],
        ),
        FunctionRisk::SensitiveRead,
        once_or_session.clone(),
    );
    let open = FunctionSpec::new(
        "open_book",
        "Open a book",
        "Starts opening the book with this id; `reading_status` reports when it is open.",
        object(
            json!({"book_id": {"type": "string", "maxLength": 32}}),
            &["book_id"],
        ),
        object(
            json!({"book_id": {"type": "string"}, "opening": {"type": "boolean"}}),
            &["book_id", "opening"],
        ),
        FunctionRisk::Mutating,
        once_or_session.clone(),
    );
    let turn = FunctionSpec::new(
        "turn_page",
        "Turn the page",
        "Moves the open document one page forward or back.",
        object(
            json!({"direction": {"type": "string", "enum": ["next", "previous"]}}),
            &["direction"],
        ),
        status_schema(),
        FunctionRisk::Mutating,
        once_or_session.clone(),
    );
    let status = FunctionSpec::new(
        "reading_status",
        "Reading status",
        "Reports whether a document is open, its title and the current page.",
        object(json!({}), &[]),
        status_schema(),
        FunctionRisk::SensitiveRead,
        once_or_session,
    );
    AppFunctionRegistry::from_functions(vec![
        AppFunction::new(list, |_| CenterCall::ListLibrary).expect("list_library spec"),
        AppFunction::typed(open, |args: BookArgs| CenterCall::OpenBook {
            book_id: args.book_id,
        })
        .expect("open_book spec"),
        AppFunction::typed(turn, |args: TurnArgs| CenterCall::TurnPage {
            forward: matches!(args.direction, Direction::Next),
        })
        .expect("turn_page spec"),
        AppFunction::new(status, |_| CenterCall::ReadingStatus).expect("reading_status spec"),
    ])
    .expect("the reader registry is valid")
}

#[must_use]
pub fn metadata() -> AppMetadata {
    AppMetadata {
        id: CENTER_APP_ID.into(),
        name: "Reader".into(),
        description: "EPUB, PDF and comic reader".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

/// The opaque id a book is listed under: a process-local digest of its path.
#[must_use]
pub fn book_id(path: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

impl App {
    // Connect to enzui-center when this build opted in and a credential is
    // enrolled for [`CENTER_APP_ID`]; otherwise stay unexposed.
    pub(crate) fn start_center(&mut self) {
        let Some(provision) = CenterProvision::discover(CENTER_APP_ID) else {
            return;
        };
        match CenterHost::spawn(provision, metadata(), registry()) {
            Ok(host) => self.center = Some(host),
            Err(error) => eprintln!("reader-app: center connector disabled: {error}"),
        }
    }

    pub(crate) fn center_subscription(&self) -> Subscription<Msg> {
        self.center
            .as_ref()
            .map_or_else(Subscription::none, |host| host.subscription(Msg::Center))
    }

    // Final shutdown: refuse new calls, cancel live ones, join the connector.
    pub(crate) fn shutdown_center(&mut self) {
        if let Some(host) = self.center.take() {
            host.shutdown();
        }
    }

    pub(crate) fn center_event(&mut self, event: ConnectorEvent) -> Task<Msg> {
        let Some(host) = self.center.as_mut() else {
            return Task::none();
        };
        // Calls are answered in this turn; no pending call-owned work survives an end.
        let CenterTurn::Call(call, request) = host.receive_with_ends(event).turn else {
            return Task::none();
        };
        let (task, outcome) = self.apply_center_call(request);
        if let Some(host) = self.center.as_mut() {
            host.answer(call, outcome);
        }
        task
    }

    fn reading_status(&self) -> Value {
        let reading = self.model.screen == Screen::Reading;
        json!({
            "reading": reading,
            "title": if reading { self.model.display_title.clone() } else { String::new() },
            "page": if reading { self.model.current_page + 1 } else { 0 },
            "page_count": if reading { self.model.page_count } else { 0 },
        })
    }

    fn apply_center_call(&mut self, request: CenterCall) -> (Task<Msg>, CenterOutcome) {
        match request {
            CenterCall::ListLibrary => {
                if self.model.library_loading {
                    return (
                        Task::none(),
                        Err(CenterFailure::new(
                            "unavailable",
                            "the library is still loading",
                        )),
                    );
                }
                let books: Vec<Value> = self
                    .model
                    .library
                    .iter()
                    .take(LIST_LIMIT)
                    .map(|book| {
                        json!({
                            "book_id": book_id(&book.path),
                            "title": book.title,
                            "author": book.author,
                            "format": book.format,
                            "progress_percentage": book.progress_percentage,
                            "available": !book.file_missing,
                        })
                    })
                    .collect();
                (Task::none(), Ok(json!({ "books": books })))
            }
            CenterCall::OpenBook { book_id: wanted } => {
                let Some(book) = self
                    .model
                    .library
                    .iter()
                    .find(|book| book_id(&book.path) == wanted)
                else {
                    return (
                        Task::none(),
                        Err(CenterFailure::new("not_found", "no book has that id")),
                    );
                };
                if book.file_missing {
                    return (
                        Task::none(),
                        Err(CenterFailure::new(
                            "unavailable",
                            "that book's file is missing",
                        )),
                    );
                }
                let path = PathBuf::from(&book.path);
                let task = self.update(Msg::OpenPath(path));
                (task, Ok(json!({ "book_id": wanted, "opening": true })))
            }
            CenterCall::TurnPage { forward } => {
                if self.model.screen != Screen::Reading {
                    return (
                        Task::none(),
                        Err(CenterFailure::new("unavailable", "no document is open")),
                    );
                }
                let task = self.update(if forward {
                    Msg::NextPage
                } else {
                    Msg::PrevPage
                });
                (task, Ok(self.reading_status()))
            }
            CenterCall::ReadingStatus => (Task::none(), Ok(self.reading_status())),
        }
    }
}
