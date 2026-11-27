//! The WebSocket echo over a real connection, with the `Origin` check
//! (contract § 3.5).

use futures_util::{SinkExt as _, StreamExt as _};
use roedy_hub_example::Example;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::{
    Message,
    client::IntoClientRequest as _,
    http::HeaderValue,
    protocol::{CloseFrame, frame::coding::CloseCode},
};

async fn start() -> std::net::SocketAddr {
    let example = Example::new("https://app.example.com/example", "http://127.0.0.1:9").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        roedy_hub_example::serve(listener, &example, std::future::pending())
            .await
            .unwrap();
    });
    addr
}

#[tokio::test]
async fn echoes_text_and_binary_then_closes() {
    let addr = start().await;
    let mut request = format!("ws://{addr}/example/ws")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "origin",
        HeaderValue::from_static("https://app.example.com"),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), 101);
    socket.send(Message::text("hello")).await.unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::text("hello")
    );
    socket
        .send(Message::binary(vec![0u8, 1, 2, 255]))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::binary(vec![0u8, 1, 2, 255])
    );
    // A clean closing handshake: the example answers the closing frame
    // before it ends the connection (a browser reports 1006 otherwise).
    socket
        .close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "done".into(),
        }))
        .await
        .unwrap();
    match socket.next().await {
        Some(Ok(Message::Close(Some(frame)))) => assert_eq!(frame.code, CloseCode::Normal),
        other => panic!("no closing frame: {other:?}"),
    }
    assert!(socket.next().await.is_none());
}

#[tokio::test]
async fn refuses_a_foreign_origin() {
    let addr = start().await;
    let mut request = format!("ws://{addr}/example/ws")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", HeaderValue::from_static("https://evil.example"));
    let error = tokio_tungstenite::connect_async(request).await.unwrap_err();
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), 403);
        }
        other => panic!("not refused: {other}"),
    }
    // Without `Origin` (not a browser) the upgrade is fine.
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/example/ws"))
        .await
        .unwrap();
    drop(socket);
}
