//! A tiny fake LLM server for developing and testing NovumChat without API keys.
//!
//! ```sh
//! cargo run -p novumchat-backend --example mock_llm -- 127.0.0.1:11999
//! ```
//!
//! * OpenAI-compatible: `GET /v1/models`, `POST /v1/chat/completions` (SSE)
//! * Ollama-compatible: `GET /api/tags`, `POST /api/show`, `POST /api/chat` (NDJSON)
//!
//! Replies depend on the last user message: mention `think`, `tool`/`files`,
//! `run`, `search`, `html`/`app`, `math` or `error` to exercise those paths.

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::Stream;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
enum Step {
    Think(String),
    Text(String),
    Tool { name: String, args: Value },
}

fn last_user(messages: &[Value]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .map(|m| match &m["content"] {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(" "),
            _ => String::new(),
        })
        .unwrap_or_default()
        .to_lowercase()
}

fn has_images(messages: &[Value]) -> bool {
    messages.iter().any(|m| {
        m["images"].as_array().is_some_and(|a| !a.is_empty())
            || m["content"].as_array().is_some_and(|parts| parts.iter().any(|p| p["type"] == "image_url"))
    })
}

fn tool_names(req: &Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .map(|t| t.iter().filter_map(|t| t["function"]["name"].as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn plan(model: &str, messages: &[Value], tools: &[String]) -> Vec<Step> {
    let tools_offered = !tools.is_empty();
    let last = messages.last().cloned().unwrap_or(Value::Null);
    let prompt = last_user(messages);
    // After a tool result: summarise it.
    if last["role"] == "tool" {
        let result = last["content"].as_str().unwrap_or("").to_string();
        let preview: String = result.chars().take(400).collect();
        return vec![Step::Text(format!(
            "I ran the tool and here is what came back:\n\n```text\n{preview}\n```\n\nEverything looks good. **Done!** ✅"
        ))];
    }
    // Title requests: answer with the first few words of the user's message,
    // like a real model would summarise it.
    if prompt.starts_with("write a short, specific title") {
        let original = last["content"].as_str().unwrap_or("");
        let user = original.split("\n\nUser: ").nth(1).unwrap_or("").split("\n\nAssistant:").next().unwrap_or("");
        let words: Vec<&str> = user.split_whitespace().take(5).collect();
        let mut title = words.join(" ").trim_end_matches(['.', '?', '!', ',']).to_string();
        if let Some(first) = title.get(..1) {
            title = first.to_uppercase() + &title[1..];
        }
        return vec![Step::Text(if title.is_empty() { "New conversation".into() } else { title })];
    }
    if prompt.contains("error") {
        return vec![Step::Text("__ERROR__".into())];
    }
    let mut steps = Vec::new();
    if prompt.contains("think") || model.contains("think") || model.contains("r1") {
        steps.push(Step::Think(
            "The user wants a careful answer. Let me break the problem into parts, check the edge cases, \
and then write a concise reply with an example."
                .into(),
        ));
    }
    if prompt.contains("mcp") {
        let mcp: Vec<&String> = tools.iter().filter(|t| t.starts_with("mcp__")).collect();
        if let Some(echo) = mcp.iter().find(|t| t.ends_with("__echo")).or(mcp.first()) {
            steps.push(Step::Text("Calling a tool from your MCP server.".into()));
            steps.push(Step::Tool {
                name: echo.to_string(),
                args: json!({ "message": "Hello from NovumChat via MCP" }),
            });
            return steps;
        }
        steps.push(Step::Text("No MCP tools are available in this chat.".into()));
        return steps;
    }
    if tools_offered && (prompt.contains("tool") || prompt.contains("files") || prompt.contains("list")) {
        steps.push(Step::Text("Let me look at the workspace first.".into()));
        steps.push(Step::Tool { name: "list_dir".into(), args: json!({ "path": ".", "depth": 1 }) });
        return steps;
    }
    if tools_offered && prompt.contains("run") {
        steps.push(Step::Text("I'll run that command for you.".into()));
        steps.push(Step::Tool {
            name: "bash".into(),
            args: json!({ "command": "echo 'Hello from NovumChat' && uname -s" }),
        });
        return steps;
    }
    if tools_offered && prompt.contains("write") {
        steps.push(Step::Tool {
            name: "write_file".into(),
            args: json!({ "path": "notes/hello.md", "content": "# Hello\n\nWritten by the mock model.\n" }),
        });
        return steps;
    }
    if tools_offered && prompt.contains("search") {
        steps.push(Step::Tool { name: "web_search".into(), args: json!({ "query": "Rust programming language" }) });
        return steps;
    }
    let label = model.to_string();
    let body = if prompt.contains("javascript") {
        "Run this snippet with the ▶ button:\n\n```js\nconst primes = [];\nfor (let n = 2; primes.length < 10; n++) {\n  if (primes.every(p => n % p)) primes.push(n);\n}\nconsole.log('First primes:', primes.join(', '));\nreturn primes.reduce((a, b) => a + b, 0);\n```\n".to_string()
    } else if prompt.contains("diagram") {
        "Here is the request flow:\n\n```mermaid\ngraph LR\n  U[You] --> C[NovumChat]\n  C --> O[Ollama]\n  C --> A[OpenAI-compatible]\n  C --> M[MCP tools]\n```\n".to_string()
    } else if prompt.contains("html") || prompt.contains("app") {
        r#"Here is a small interactive app — it runs live in a sandbox:

```html
<!doctype html>
<html>
<head>
<title>Particle Garden</title>
<style>
  html,body{margin:0;height:100%;background:#0b1020;overflow:hidden;font-family:system-ui}
  canvas{display:block}
  .hint{position:fixed;left:12px;bottom:10px;color:#9fb3ff;font-size:13px;opacity:.8}
</style>
</head>
<body>
<canvas id="c"></canvas>
<div class="hint">Move or tap to spawn particles</div>
<script>
const c = document.getElementById('c'), x = c.getContext('2d');
let W, H, P = [];
function size(){ W = c.width = innerWidth; H = c.height = innerHeight; }
addEventListener('resize', size); size();
function spawn(px, py){ for(let i=0;i<12;i++) P.push({x:px,y:py,vx:(Math.random()-.5)*4,vy:(Math.random()-.5)*4,l:1,h:200+Math.random()*120}); }
addEventListener('pointermove', e => spawn(e.clientX, e.clientY));
addEventListener('pointerdown', e => spawn(e.clientX, e.clientY));
(function loop(){
  x.fillStyle = 'rgba(11,16,32,.18)'; x.fillRect(0,0,W,H);
  if (Math.random() < .3) spawn(Math.random()*W, Math.random()*H);
  P = P.filter(p => (p.l -= .012) > 0);
  for (const p of P){ p.x+=p.vx; p.y+=p.vy; p.vy+=.03; x.beginPath(); x.arc(p.x,p.y,3*p.l+1,0,7); x.fillStyle=`hsla(${p.h},90%,65%,${p.l})`; x.fill(); }
  requestAnimationFrame(loop);
})();
</script>
</body>
</html>
```

Try moving your pointer over it!"#
            .to_string()
    } else if prompt.contains("math") {
        r#"Here's the **Gaussian integral**, a classic:

$$\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}$$

And Euler's identity inline: $e^{i\pi} + 1 = 0$. With bracket delimiters: \(a^2 + b^2 = c^2\).

\[
\mathbf{F} = m\mathbf{a} = m\frac{d^2\mathbf{x}}{dt^2}
\]

A system written as a bare environment:

\begin{align}
(a + b)^2 &= a^2 + 2ab + b^2 \\
- (a - b)^2 &= -a^2 + 2ab - b^2 % the signs flip
\end{align}

Chemistry too: $\ce{2H2 + O2 -> 2H2O}$, and MathTeX: <script type="math/tex">\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}</script>.

| Symbol | Meaning |
|---|---|
| $\pi$ | ratio of circumference to diameter |
| $e$ | Euler's number |
"#
        .to_string()
    } else {
        format!(
            "Hi! I'm **{label}**, a mock model streaming this reply token by token.\n\n\
Here's some *Markdown* to show off the renderer:\n\n\
- Lists with `inline code`\n- **Bold** and ~~strikethrough~~\n- Math like $E = mc^2$\n\n\
```rust\nfn main() {{\n    println!(\"Hello, NovumChat!\");\n}}\n```\n\n\
> [!TIP]\n> Ask for an *html app*, some *math*, to *list files* or to *search*.\n{}",
            if has_images(messages) { "\nI can see the image you attached. 🖼️\n" } else { "" }
        )
    };
    steps.push(Step::Text(body));
    steps
}

/// Splits text into small streaming chunks (≈ words).
fn chunks(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        cur.push(ch);
        if ch == ' ' || ch == '\n' || cur.len() > 12 {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn delay(model: &str) -> Duration {
    if model.contains("fast") { Duration::from_millis(4) } else { Duration::from_millis(22) }
}

// ── OpenAI ─────────────────────────────────────────────────────────────────

async fn openai_models() -> Json<Value> {
    Json(json!({ "object": "list", "data": [
        { "id": "mock-gpt", "object": "model", "owned_by": "mock" },
        { "id": "mock-claude", "object": "model", "owned_by": "mock" },
        { "id": "mock-fast", "object": "model", "owned_by": "mock" },
        { "id": "mock-r1-think", "object": "model", "owned_by": "mock" }
    ]}))
}

async fn openai_chat(Json(req): Json<Value>) -> Response {
    let model = req["model"].as_str().unwrap_or("mock").to_string();
    let messages = req["messages"].as_array().cloned().unwrap_or_default();
    let steps = plan(&model, &messages, &tool_names(&req));
    if matches!(steps.first(), Some(Step::Text(t)) if t == "__ERROR__") {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "Mock error: the model refused (you asked for an error)."}})),
        )
            .into_response();
    }
    let stream = openai_stream(model, steps);
    Sse::new(stream).into_response()
}

fn openai_stream(model: String, steps: Vec<Step>) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let d = delay(&model);
        let mk = |delta: Value, finish: Value| json!({
            "id": "chatcmpl-mock", "object": "chat.completion.chunk", "model": model,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
        });
        yield Ok(Event::default().data(mk(json!({"role": "assistant", "content": ""}), Value::Null).to_string()));
        let mut finish = "stop";
        let mut tool_index = 0;
        for step in steps {
            match step {
                Step::Think(t) => for c in chunks(&t) {
                    tokio::time::sleep(d).await;
                    yield Ok(Event::default().data(mk(json!({"reasoning_content": c}), Value::Null).to_string()));
                },
                Step::Text(t) => for c in chunks(&t) {
                    tokio::time::sleep(d).await;
                    yield Ok(Event::default().data(mk(json!({"content": c}), Value::Null).to_string()));
                },
                Step::Tool { name, args } => {
                    finish = "tool_calls";
                    let args = args.to_string();
                    let (a, b) = args.split_at(args.len() / 2);
                    yield Ok(Event::default().data(mk(json!({"tool_calls": [{"index": tool_index, "id": format!("call_mock_{tool_index}"), "type": "function", "function": {"name": name, "arguments": a}}]}), Value::Null).to_string()));
                    tokio::time::sleep(d).await;
                    yield Ok(Event::default().data(mk(json!({"tool_calls": [{"index": tool_index, "function": {"arguments": b}}]}), Value::Null).to_string()));
                    tool_index += 1;
                }
            }
        }
        yield Ok(Event::default().data(mk(json!({}), json!(finish)).to_string()));
        yield Ok(Event::default().data(json!({"id": "chatcmpl-mock", "object": "chat.completion.chunk", "choices": [], "usage": {"prompt_tokens": 42, "completion_tokens": 128, "total_tokens": 170}}).to_string()));
        yield Ok(Event::default().data("[DONE]"));
    }
}

// ── Ollama ─────────────────────────────────────────────────────────────────

async fn ollama_tags() -> Json<Value> {
    Json(json!({ "models": [
        { "name": "mock-llama:8b", "model": "mock-llama:8b", "modified_at": "2026-01-01T00:00:00Z", "size": 4_900_000_000u64, "digest": "x" },
        { "name": "mock-vision:4b", "model": "mock-vision:4b", "modified_at": "2026-01-01T00:00:00Z", "size": 3_300_000_000u64, "digest": "y" }
    ]}))
}

async fn ollama_show(Json(req): Json<Value>) -> Json<Value> {
    let name = req["model"].as_str().unwrap_or("");
    let mut caps = vec!["completion", "tools"];
    if name.contains("vision") {
        caps.push("vision");
    }
    if name.contains("llama") {
        caps.push("thinking");
    }
    Json(json!({ "capabilities": caps, "modelfile": "", "parameters": "", "template": "" }))
}

async fn ollama_chat(Json(req): Json<Value>) -> Response {
    let model = req["model"].as_str().unwrap_or("mock").to_string();
    let messages = req["messages"].as_array().cloned().unwrap_or_default();
    let mut steps = plan(&model, &messages, &tool_names(&req));
    if req["think"].as_bool() == Some(true) && !steps.iter().any(|s| matches!(s, Step::Think(_))) {
        steps.insert(0, Step::Think("Thinking natively through Ollama's `think` option…".into()));
    }
    if matches!(steps.first(), Some(Step::Text(t)) if t == "__ERROR__") {
        return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error": "mock error requested"}))).into_response();
    }
    let stream = async_stream::stream! {
        let d = delay(&model);
        let line = |msg: Value, done: bool| {
            let mut v = json!({ "model": model, "created_at": "2026-01-01T00:00:00Z", "message": msg, "done": done });
            if done {
                v["done_reason"] = json!("stop");
                for (k, n) in [("total_duration", 1u64), ("load_duration", 1), ("prompt_eval_count", 37), ("prompt_eval_duration", 1), ("eval_count", 99), ("eval_duration", 1)] {
                    v[k] = json!(n);
                }
            }
            Ok::<_, Infallible>(format!("{v}\n"))
        };
        for step in steps {
            match step {
                Step::Think(t) => for c in chunks(&t) {
                    tokio::time::sleep(d).await;
                    yield line(json!({"role": "assistant", "content": "", "thinking": c}), false);
                },
                Step::Text(t) => for c in chunks(&t) {
                    tokio::time::sleep(d).await;
                    yield line(json!({"role": "assistant", "content": c}), false);
                },
                Step::Tool { name, args } => {
                    yield line(json!({"role": "assistant", "content": "", "tool_calls": [{"function": {"name": name, "arguments": args}}]}), false);
                }
            }
        }
        yield line(json!({"role": "assistant", "content": ""}), true);
    };
    Response::builder().header("content-type", "application/x-ndjson").body(Body::from_stream(stream)).unwrap()
}

#[tokio::main]
async fn main() {
    let addr = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:11999".into());
    let app = Router::new()
        .route("/v1/models", get(openai_models))
        .route("/v1/chat/completions", post(openai_chat))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/api/chat", post(ollama_chat));
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    println!("mock LLM server on http://{addr}  (OpenAI: /v1, Ollama: /api)");
    axum::serve(listener, app).await.unwrap();
}
