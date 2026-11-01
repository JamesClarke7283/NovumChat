//! Bindings to `public/js/bridge.js` (`window.NovumBridge`).

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = NovumBridge, js_name = isTauri)]
    pub fn is_tauri() -> bool;

    #[wasm_bindgen(js_namespace = NovumBridge, catch)]
    pub fn invoke(method: &str, params: &str) -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(js_namespace = NovumBridge)]
    pub fn listen(cb: &Closure<dyn FnMut(String)>);

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = renderMath)]
    pub fn render_math(tex: &str, display: bool) -> Option<String>;

    #[wasm_bindgen(js_namespace = NovumBridge)]
    pub fn highlight(code: &str, lang: &str) -> Option<String>;

    #[wasm_bindgen(js_namespace = NovumBridge)]
    pub fn sanitize(html: &str, allow_remote: bool) -> String;

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = sanitizeSvg)]
    pub fn sanitize_svg(svg: &str) -> String;

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = artifactDoc)]
    pub fn artifact_doc(code: &str, kind: &str, allow_network: bool, dark: bool) -> String;

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = jsRunnerDoc)]
    pub fn js_runner_doc(code: &str, dark: bool) -> String;

    #[wasm_bindgen(js_namespace = NovumBridge)]
    pub fn mermaid(code: &str, dark: bool) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = copyText)]
    pub fn copy_text(text: &str) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = readFileBase64)]
    pub fn read_file_base64(file: &web_sys::File) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = objectUrl)]
    pub fn object_url(blob: &web_sys::Blob) -> String;

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = revokeUrl)]
    pub fn revoke_url(url: &str);

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = dataUrlToObjectUrl)]
    pub fn b64_to_object_url(b64: &str, mime: &str) -> String;

    #[wasm_bindgen(js_namespace = NovumBridge)]
    pub fn download(filename: &str, content: &str, mime: &str);

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = canSpeak)]
    pub fn can_speak() -> bool;

    #[wasm_bindgen(js_namespace = NovumBridge)]
    pub fn speak(text: &str, on_end: &Closure<dyn FnMut()>) -> bool;

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = stopSpeaking)]
    pub fn stop_speaking();

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = prefersDark)]
    pub fn prefers_dark() -> bool;

    #[wasm_bindgen(js_namespace = NovumBridge, js_name = onSchemeChange)]
    pub fn on_scheme_change(cb: &Closure<dyn FnMut(bool)>);

    #[wasm_bindgen(js_namespace = NovumBridge)]
    pub fn store(key: &str, value: &str);

    #[wasm_bindgen(js_namespace = NovumBridge)]
    pub fn load(key: &str) -> Option<String>;

    #[wasm_bindgen(js_namespace = NovumBridge)]
    pub fn vibrate(ms: u32);
}

/// Awaits a JS promise and converts errors to readable strings.
pub async fn await_promise(p: js_sys::Promise) -> Result<JsValue, String> {
    wasm_bindgen_futures::JsFuture::from(p).await.map_err(js_error)
}

pub fn js_error(v: JsValue) -> String {
    if let Some(s) = v.as_string() {
        return s;
    }
    if let Some(e) = v.dyn_ref::<js_sys::Error>() {
        return String::from(e.message());
    }
    format!("{v:?}")
}

pub async fn copy(text: &str) -> bool {
    await_promise(copy_text(text)).await.ok().and_then(|v| v.as_bool()).unwrap_or(false)
}
