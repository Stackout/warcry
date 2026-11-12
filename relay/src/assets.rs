const INDEX: &str = include_str!("../ui/index.html");
const APP: &str = include_str!("../ui/app.js");
const STYLE: &str = include_str!("../ui/style.css");
const MANIFEST: &str = include_str!("../ui/manifest.json");

pub fn serve(path: &str) -> Vec<u8> {
    match asset(path) {
        Some((kind, body)) => response("200 OK", kind, body),
        None => response("404 Not Found", "text/plain; charset=utf-8", "not found"),
    }
}

fn asset(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/" | "/index.html" => Some(("text/html; charset=utf-8", INDEX)),
        "/app.js" => Some(("text/javascript; charset=utf-8", APP)),
        "/style.css" => Some(("text/css; charset=utf-8", STYLE)),
        "/manifest.json" => Some(("application/manifest+json", MANIFEST)),
        _ => None,
    }
}

fn response(status: &str, kind: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(path: &str) -> String {
        String::from_utf8(serve(path)).unwrap()
    }

    #[test]
    fn the_root_serves_the_page() {
        let response = text("/");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("text/html"));
        assert!(response.contains("<!doctype html>"));
    }

    #[test]
    fn every_asset_the_page_references_is_served() {
        for path in ["/app.js", "/style.css", "/manifest.json"] {
            assert!(text(path).starts_with("HTTP/1.1 200 OK"), "missing {path}");
        }
    }

    #[test]
    fn an_unknown_path_is_a_clean_404() {
        assert!(text("/../../etc/passwd").starts_with("HTTP/1.1 404"));
        assert!(text("/wp-admin").starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn content_length_matches_the_body() {
        let response = text("/style.css");
        let declared: usize = response
            .split_once("Content-Length: ")
            .map(|split| split.1)
            .and_then(|rest| rest.split("\r\n").next())
            .and_then(|n| n.parse().ok())
            .unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1;
        assert_eq!(declared, body.len());
    }
}
