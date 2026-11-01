//! Tool calls inside assistant messages, and approval requests.

use leptos::prelude::*;
use novumchat_core::permissions::ToolRisk;
use novumchat_core::types::*;
use novumchat_core::util::{format_duration_ms, truncate_chars};
use serde_json::Value;

use crate::icons::Icon;
use crate::state::use_state;
use crate::util::host_of;

fn risk_str(r: Option<ToolRisk>) -> &'static str {
    match r {
        Some(ToolRisk::Read) | None => "read",
        Some(ToolRisk::Search) => "search",
        Some(ToolRisk::Network) => "network",
        Some(ToolRisk::Write) => "write",
        Some(ToolRisk::Execute) => "execute",
        Some(ToolRisk::External) => "external",
    }
}

fn tool_icon(name: &str) -> &'static str {
    match name {
        "bash" => "terminal",
        "read_file" => "file-text",
        "write_file" | "edit_file" => "file-edit",
        "list_dir" => "folder",
        "search_files" => "search",
        "web_search" => "globe",
        "fetch_url" => "external",
        _ => "server",
    }
}

fn arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(Value::as_str).map(|s| truncate_chars(s.lines().next().unwrap_or(s), 90)).unwrap_or_default()
}

/// Human description like "Ran `ls -la`" (past) / "Running `ls -la`" (present).
fn describe(tc: &ToolCallPart) -> (String, String) {
    let args = tc.arguments_json();
    let done = tc.status.is_finished();
    let verb = |past: &str, present: &str| if done { past.to_string() } else { present.to_string() };
    match tc.name.as_str() {
        "bash" => (verb("Ran", "Running"), arg(&args, "command")),
        "read_file" => (verb("Read", "Reading"), arg(&args, "path")),
        "write_file" => (verb("Wrote", "Writing"), arg(&args, "path")),
        "edit_file" => (verb("Edited", "Editing"), arg(&args, "path")),
        "list_dir" => (verb("Listed", "Listing"), {
            let p = arg(&args, "path");
            if p.is_empty() { "workspace".into() } else { p }
        }),
        "search_files" => (verb("Searched files for", "Searching files for"), arg(&args, "pattern")),
        "web_search" => (verb("Searched the web for", "Searching the web for"), arg(&args, "query")),
        "fetch_url" => (verb("Read", "Reading"), host_of(&arg(&args, "url"))),
        other => {
            let pretty = other.strip_prefix("mcp__").map(|rest| {
                let mut it = rest.splitn(2, "__");
                let server = it.next().unwrap_or("");
                let tool = it.next().unwrap_or(rest);
                format!("{tool} on {server}")
            });
            (verb("Used", "Using"), pretty.unwrap_or_else(|| other.to_string()))
        }
    }
}

#[component]
pub fn ToolCallView(sig: ArcRwSignal<Message>, index: usize) -> impl IntoView {
    let state = use_state();
    let part = Memo::new(move |_| {
        sig.with(|m| match m.parts.get(index) {
            Some(Part::ToolCall(tc)) => Some(tc.clone()),
            _ => None,
        })
    });
    let open = RwSignal::new(false);
    let approval = Memo::new(move |_| {
        let id = part.with(|p| p.as_ref().map(|p| p.id.clone()))?;
        state.approvals.with(|a| a.iter().find(|r| r.tool_call_id == id).cloned())
    });

    view! {
        {move || part.get().map(|tc| {
            let (verb, object) = describe(&tc);
            let risk = risk_str(tc.risk);
            let status = tc.status;
            let state_view = match status {
                ToolCallStatus::Pending | ToolCallStatus::Running => view! {
                    <span class="nc-tool-state"><span class="nc-spinner"></span>"Running"</span>
                }.into_any(),
                ToolCallStatus::AwaitingApproval => view! {
                    <span class="nc-tool-state" style="color:var(--warn)"><Icon name="shield" size=14/>"Needs approval"</span>
                }.into_any(),
                ToolCallStatus::Success => view! {
                    <span class="nc-tool-state ok"><Icon name="check" size=14/>{tc.duration_ms.map(format_duration_ms).unwrap_or_default()}</span>
                }.into_any(),
                ToolCallStatus::Error => view! {
                    <span class="nc-tool-state err"><Icon name="x-circle" size=14/>"Failed"</span>
                }.into_any(),
                ToolCallStatus::Denied => view! {
                    <span class="nc-tool-state err"><Icon name="ban" size=14/>"Declined"</span>
                }.into_any(),
            };
            let args_pretty = serde_json::to_string_pretty(&tc.arguments_json()).unwrap_or_else(|_| tc.arguments.clone());
            let result = tc.result.clone().unwrap_or_default();
            let sources: Vec<SearchResult> = tc
                .data
                .as_ref()
                .and_then(|d| d.get("results"))
                .and_then(|r| serde_json::from_value(r.clone()).ok())
                .unwrap_or_default();
            let has_sources = !sources.is_empty();
            view! {
                <div class="nc-tool" data-risk=risk>
                    <button class="nc-tool-head" aria-expanded=move || open.get().to_string() on:click=move |_| open.update(|o| *o = !*o)>
                        <span class="nc-tool-icon"><Icon name=tool_icon(&tc.name) size=16/></span>
                        <span class="nc-tool-title">{verb}" "<code>{object}</code></span>
                        {state_view}
                        <Icon name="chevron-down" size=15/>
                    </button>
                    {has_sources.then(|| view! {
                        <div class="nc-sources">
                            {sources.into_iter().enumerate().map(|(i, s)| {
                                let host = host_of(&s.url);
                                let url = s.url.clone();
                                view! {
                                    <button class="nc-source" title=s.url.clone() on:click=move |_| state.open_link(url.clone())>
                                        <b>{s.title.clone()}</b>
                                        <small><span class="fav" title="Citation number">{(i + 1).to_string()}</span><span class="nc-truncate">{host}</span></small>
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                    })}
                    <Show when=move || open.get()>
                        <div class="nc-tool-body">
                            <div class="nc-tool-section">
                                <h5>"Arguments"</h5>
                                <pre>{args_pretty.clone()}</pre>
                            </div>
                            {(!result.is_empty()).then(|| view! {
                                <div class="nc-tool-section">
                                    <h5>"Result"</h5>
                                    <pre>{result.clone()}</pre>
                                </div>
                            })}
                        </div>
                    </Show>
                </div>
            }
        })}
        {move || approval.get().map(|req| view! { <ApprovalCard request=req/> })}
    }
}

fn detail_lines(detail: &str) -> impl IntoView + use<> {
    detail
        .lines()
        .map(|l| {
            let class = if l.starts_with("+++") || l.starts_with("---") {
                "hunk"
            } else if l.starts_with('+') {
                "add"
            } else if l.starts_with('-') {
                "del"
            } else if l.starts_with("@@") {
                "hunk"
            } else {
                ""
            };
            view! { <span class=class>{l.to_string()}"\n"</span> }
        })
        .collect_view()
}

#[component]
pub fn ApprovalCard(request: ApprovalRequest) -> impl IntoView {
    let state = use_state();
    let risk = risk_str(Some(request.risk));
    let what = match request.risk {
        ToolRisk::Execute => "This runs a command on your computer.",
        ToolRisk::Write => "This changes files on your computer.",
        ToolRisk::Network => "This downloads a page from the internet.",
        ToolRisk::External => "This calls a tool provided by an MCP server.",
        ToolRisk::Read => "This reads files outside the workspace.",
        ToolRisk::Search => "This sends a query to your search engine.",
    };
    let id_once = request.id.clone();
    let id_chat = request.id.clone();
    let id_deny = request.id.clone();
    view! {
        <div class="nc-approval" data-risk=risk role="alertdialog" aria-label=request.title.clone()>
            <div class="nc-approval-head">
                <span class="nc-tool-icon"><Icon name=tool_icon(&request.tool) size=17/></span>
                <div style="min-width:0">
                    <div class="nc-approval-title">{request.title.clone()}</div>
                    <div class="nc-approval-sub">{what}</div>
                </div>
            </div>
            <div class="nc-approval-detail">{detail_lines(&request.detail)}</div>
            <div class="nc-approval-actions">
                <button class="nc-btn nc-btn-primary" on:click=move |_| state.resolve_approval(id_once.clone(), ApprovalDecision::Allow)>
                    <Icon name="check" size=16/>
                    "Allow once"
                </button>
                <button class="nc-btn nc-btn-secondary" on:click=move |_| state.resolve_approval(id_chat.clone(), ApprovalDecision::AllowForChat)>
                    "Always allow in this chat"
                </button>
                <button class="nc-btn nc-btn-danger-ghost" on:click=move |_| state.resolve_approval(id_deny.clone(), ApprovalDecision::Deny)>
                    "Deny"
                </button>
            </div>
        </div>
    }
}
