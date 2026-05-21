use futures_util::{SinkExt, StreamExt};
use plexus_common::protocol::{HelloCaps, HelloFrame, PongFrame, WsFrame};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use uuid::Uuid;

pub struct DeviceClient {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl DeviceClient {
    pub async fn connect(base: &str, token: Option<&str>) -> Self {
        Self::connect_path(base, "/ws/device", token).await
    }

    pub async fn connect_path(base: &str, path: &str, token: Option<&str>) -> Self {
        let mut req = format!("{base}{path}").into_client_request().unwrap();
        if let Some(token) = token {
            req.headers_mut()
                .insert("Authorization", format!("Bearer {token}").parse().unwrap());
        }
        let (ws, _) = connect_async(req).await.unwrap();
        Self { ws }
    }

    pub async fn send_hello(&mut self, version: &str) -> Uuid {
        let id = Uuid::now_v7();
        self.send(WsFrame::Hello(HelloFrame {
            id,
            version: version.to_string(),
            client_version: "test-client".to_string(),
            os: "linux".to_string(),
            caps: HelloCaps {
                sandbox: "none".to_string(),
                exec: false,
                fs: "rw".to_string(),
            },
        }))
        .await;
        id
    }

    pub async fn send(&mut self, frame: WsFrame) {
        let text = serde_json::to_string(&frame).unwrap();
        self.ws.send(Message::Text(text.into())).await.unwrap();
    }

    pub async fn recv_frame(&mut self) -> WsFrame {
        loop {
            match self.next_message().await {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(payload) => self.ws.send(Message::Pong(payload)).await.unwrap(),
                other => panic!("unexpected websocket message: {other:?}"),
            }
        }
    }

    pub async fn recv_close_code(&mut self) -> u16 {
        loop {
            match self.next_message().await {
                Message::Close(Some(frame)) => return frame.code.into(),
                Message::Close(None) => return 1005,
                Message::Text(_) => continue,
                other => panic!("unexpected websocket message: {other:?}"),
            }
        }
    }

    pub async fn reply_pong(&mut self, id: Uuid) {
        self.send(WsFrame::Pong(PongFrame { id })).await;
    }

    async fn next_message(&mut self) -> Message {
        timeout(Duration::from_secs(5), self.ws.next())
            .await
            .expect("timed out waiting for websocket message")
            .expect("websocket stream ended")
            .expect("websocket read failed")
    }
}
