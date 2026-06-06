use crate::proxy::ResBody;
use http_body_util::BodyExt;
use hyper::{Request, Response, body::Incoming, header};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

/// Headers that must not be forwarded between hops (RFC 7230 §6.1).
static HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

/// Forward a plain-HTTP (non-CONNECT) proxy request to the origin server and
/// return the response for delivery to the browser.
pub async fn forward_http(req: Request<Incoming>) -> anyhow::Result<Response<ResBody>> {
    let (mut parts, body) = req.into_parts();

    let authority = parts
        .uri
        .authority()
        .ok_or_else(|| anyhow::anyhow!("request URI has no authority: {}", parts.uri))?
        .clone();

    let host = authority.host().to_owned();
    let port = authority.port_u16().unwrap_or(80);

    tracing::debug!(method = %parts.method, host = %host, port, "forwarding");

    // Rewrite to origin-form: strip scheme + authority, keep path + query.
    let path = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_owned();
    parts.uri = path.parse()?;

    // Ensure Host header is present.
    parts
        .headers
        .insert(header::HOST, authority.as_str().parse()?);

    strip_hop_by_hop(&mut parts.headers);

    let origin_req = Request::from_parts(parts, body);

    let stream = TcpStream::connect((host.as_str(), port)).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
        .handshake(io)
        .await?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::debug!("origin connection driver error: {e}");
        }
    });

    let origin_res = sender.send_request(origin_req).await?;
    let (mut res_parts, res_body) = origin_res.into_parts();

    strip_hop_by_hop(&mut res_parts.headers);

    Ok(Response::from_parts(res_parts, res_body.boxed()))
}

fn strip_hop_by_hop(headers: &mut hyper::HeaderMap) {
    // First collect extra names listed in the Connection header.
    let extra: Vec<String> = headers
        .get_all(header::CONNECTION)
        .iter()
        .flat_map(|v| v.to_str().unwrap_or("").split(','))
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();

    for name in HOP_BY_HOP {
        headers.remove(*name);
    }
    for name in &extra {
        headers.remove(name.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header;

    #[test]
    fn strips_standard_hop_by_hop() {
        let mut map = hyper::HeaderMap::new();
        map.insert(header::CONNECTION, "keep-alive".parse().unwrap());
        map.insert("keep-alive", "timeout=5".parse().unwrap());
        map.insert(header::HOST, "example.com".parse().unwrap());

        strip_hop_by_hop(&mut map);

        assert!(!map.contains_key("keep-alive"));
        assert!(!map.contains_key(header::CONNECTION));
        assert!(map.contains_key(header::HOST));
    }

    #[test]
    fn strips_connection_header_named_extensions() {
        let mut map = hyper::HeaderMap::new();
        map.insert(header::CONNECTION, "x-custom-header".parse().unwrap());
        map.insert("x-custom-header", "value".parse().unwrap());
        map.insert(header::CONTENT_TYPE, "text/html".parse().unwrap());

        strip_hop_by_hop(&mut map);

        assert!(!map.contains_key("x-custom-header"));
        assert!(!map.contains_key(header::CONNECTION));
        assert!(map.contains_key(header::CONTENT_TYPE));
    }

    #[test]
    fn strips_multiple_comma_separated_connection_extensions() {
        let mut map = hyper::HeaderMap::new();
        map.insert(
            header::CONNECTION,
            "keep-alive, x-foo, x-bar".parse().unwrap(),
        );
        map.insert("x-foo", "1".parse().unwrap());
        map.insert("x-bar", "2".parse().unwrap());
        map.insert(header::CONTENT_LENGTH, "42".parse().unwrap());

        strip_hop_by_hop(&mut map);

        assert!(!map.contains_key(header::CONNECTION));
        assert!(!map.contains_key("keep-alive"));
        assert!(!map.contains_key("x-foo"));
        assert!(!map.contains_key("x-bar"));
        assert!(map.contains_key(header::CONTENT_LENGTH));
    }
}
