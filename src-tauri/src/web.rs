use tauri::http;

const INDEX: &[u8] = include_bytes!("../../frontend/index.html");
const STYLES: &[u8] = include_bytes!("../../frontend/styles.css");
const APP: &[u8] = include_bytes!("../../frontend/app.js");
const GS_ICON: &[u8] = include_bytes!("../../frontend/assets/images/gs.png");
const PRIMO_ICON: &[u8] = include_bytes!("../../frontend/assets/images/primo.png");
const NL_ICON: &[u8] = include_bytes!("../../frontend/assets/images/nl.png");

pub fn response(request: http::Request<Vec<u8>>) -> http::Response<&'static [u8]> {
    let (content_type, body) = match request.uri().path().trim_start_matches('/') {
        "" | "index.html" => ("text/html; charset=utf-8", INDEX),
        "styles.css" => ("text/css; charset=utf-8", STYLES),
        "app.js" => ("application/javascript; charset=utf-8", APP),
        "assets/images/gs.png" => ("image/png", GS_ICON),
        "assets/images/primo.png" => ("image/png", PRIMO_ICON),
        "assets/images/nl.png" => ("image/png", NL_ICON),
        _ => ("text/html; charset=utf-8", INDEX),
    };

    http::Response::builder()
        .header(http::header::CONTENT_TYPE, content_type)
        .body(body)
        .expect("failed to build frontend response")
}
