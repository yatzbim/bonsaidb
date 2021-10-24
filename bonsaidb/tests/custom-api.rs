//! Tests invoking an API defined in a custom backend.

use bonsaidb::{
    client::{url::Url, Client},
    core::{
        custom_api::CustomApi,
        permissions::{Actionable, Dispatcher, Permissions},
        test_util::{Basic, TestDirectory},
    },
    server::{
        Backend, BackendError, Configuration, ConnectedClient, CustomServer, DefaultPermissions,
    },
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Dispatcher)]
#[dispatcher(input = CustomRequest)]
struct CustomBackend;

impl Backend for CustomBackend {
    type CustomApi = Self;

    type CustomApiDispatcher = Self;

    fn dispatcher_for(
        _server: &CustomServer<Self>,
        _client: &ConnectedClient<Self>,
    ) -> Self::CustomApiDispatcher {
        CustomBackend
    }
}

impl CustomApi for CustomBackend {
    type Request = CustomRequest;
    type Response = CustomResponse;
    type Error = ();
}

#[derive(Serialize, Deserialize, Debug, Actionable)]
enum CustomRequest {
    #[actionable(protection = "none")]
    Ping,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum CustomResponse {
    Pong,
}

#[tokio::test]
async fn custom_api() -> anyhow::Result<()> {
    let dir = TestDirectory::new("custom_api.bonsaidb");
    let server = CustomServer::<CustomBackend>::open(
        dir.as_ref(),
        Configuration {
            default_permissions: DefaultPermissions::AllowAll,
            ..Configuration::default()
        },
    )
    .await?;
    server
        .install_self_signed_certificate("test", false)
        .await?;
    let certificate = server.certificate().await?;
    server.register_schema::<Basic>().await?;
    tokio::spawn(async move { server.listen_on(12346).await });

    let client = Client::build(Url::parse("bonsaidb://localhost:12346")?)
        .with_custom_api::<CustomBackend>()
        .with_certificate(certificate)
        .finish()
        .await?;

    let CustomResponse::Pong = client.send_api_request(CustomRequest::Ping).await?.unwrap();

    Ok(())
}

impl CustomRequestDispatcher for CustomBackend {
    type Output = CustomResponse;
    type Error = BackendError<()>;
}

#[actionable::async_trait]
impl PingHandler for CustomBackend {
    async fn handle(&self, _permissions: &Permissions) -> Result<CustomResponse, BackendError<()>> {
        Ok(CustomResponse::Pong)
    }
}
