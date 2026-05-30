use axum::{
    body::Body,
    extract::Path,
    http::{header, HeaderValue, StatusCode},
    response::Response,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist/"]
struct Assets;

/// 服务前端静态资源；找不到时回退 index.html（SPA 路由）。
pub async fn handler(path: Option<Path<String>>) -> Response {
    let p = path.map(|p| p.0).unwrap_or_default();
    serve(&p).unwrap_or_else(|| serve("index.html").unwrap_or_else(not_found))
}

fn serve(p: &str) -> Option<Response> {
    let file = Assets::get(p)?;
    let mime = mime_guess::from_path(p).first_or_octet_stream();
    let mut resp = Response::new(Body::from(file.data));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref()).ok()?,
    );
    Some(resp)
}

fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from("not found"))
        .unwrap()
}
