/// Bidirectional raw byte relay between a browser-facing and an origin-facing stream.
///
/// This is a placeholder that will be replaced in the next step with a proper
/// HTTP/1.1 loop that feeds requests and responses through the scan hooks.
pub async fn run<B, O>(browser: B, origin: O) -> anyhow::Result<()>
where
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite,
    O: tokio::io::AsyncRead + tokio::io::AsyncWrite,
{
    let (mut br, mut bw) = tokio::io::split(browser);
    let (mut or_, mut ow) = tokio::io::split(origin);

    // Run both copy directions concurrently; the tunnel ends when either side
    // closes the connection.
    tokio::select! {
        _ = tokio::io::copy(&mut br, &mut ow) => {},
        _ = tokio::io::copy(&mut or_, &mut bw) => {},
    }

    Ok(())
}
