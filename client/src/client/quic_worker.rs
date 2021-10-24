use std::sync::Arc;

use bonsaidb_core::{
    custom_api::{CustomApi, CustomApiResult},
    networking::{Payload, Request, Response},
};
use fabruic::{self, Certificate, Endpoint};
use flume::Receiver;
use futures::StreamExt;
use url::Url;

use super::{CustomApiCallback, PendingRequest};
#[cfg(feature = "pubsub")]
use crate::client::SubscriberMap;
use crate::{client::OutstandingRequestMapHandle, Error};

/// This function will establish a connection and try to keep it active. If an
/// error occurs, any queries that come in while reconnecting will have the
/// error replayed to them.
pub async fn reconnecting_client_loop<A: CustomApi>(
    mut url: Url,
    certificate: Option<Certificate>,
    request_receiver: Receiver<PendingRequest<A::Request, CustomApiResult<A>>>,
    custom_api_callback: Option<Arc<dyn CustomApiCallback<A>>>,
    #[cfg(feature = "pubsub")] subscribers: SubscriberMap,
) -> Result<(), Error> {
    if url.port().is_none() && url.scheme() == "bonsaidb" {
        let _ = url.set_port(Some(5645));
    }

    while let Ok(request) = request_receiver.recv_async().await {
        if let Err((failed_request, err)) = connect_and_process(
            &url,
            certificate.as_ref(),
            request,
            &request_receiver,
            custom_api_callback.clone(),
            #[cfg(feature = "pubsub")]
            &subscribers,
        )
        .await
        {
            if let Some(failed_request) = failed_request {
                drop(failed_request.responder.send(Err(err)));
            } else {
                // TODO this can result in an infinite loop
                println!(
                    "Received an error: {:?} with no response to report the error to",
                    err,
                );
            }
            continue;
        }
    }

    Ok(())
}

async fn connect_and_process<A: CustomApi>(
    url: &Url,
    certificate: Option<&Certificate>,
    initial_request: PendingRequest<A::Request, CustomApiResult<A>>,
    request_receiver: &Receiver<PendingRequest<A::Request, CustomApiResult<A>>>,
    custom_api_callback: Option<Arc<dyn CustomApiCallback<A>>>,
    #[cfg(feature = "pubsub")] subscribers: &SubscriberMap,
) -> Result<
    (),
    (
        Option<PendingRequest<A::Request, CustomApiResult<A>>>,
        Error,
    ),
> {
    let (_connection, payload_sender, payload_receiver) = match connect::<A>(url, certificate).await
    {
        Ok(result) => result,
        Err(err) => return Err((Some(initial_request), err)),
    };

    let outstanding_requests = OutstandingRequestMapHandle::default();
    let request_processor = tokio::spawn(process(
        outstanding_requests.clone(),
        payload_receiver,
        custom_api_callback,
        #[cfg(feature = "pubsub")]
        subscribers.clone(),
    ));

    if let Err(err) = payload_sender.send(&initial_request.request) {
        return Err((Some(initial_request), Error::from(err)));
    }

    {
        let mut outstanding_requests = outstanding_requests.lock().await;
        outstanding_requests.insert(
            initial_request
                .request
                .id
                .expect("all requests require ids"),
            initial_request,
        );
    }

    futures::try_join!(
        process_requests::<A>(outstanding_requests, request_receiver, payload_sender),
        async { request_processor.await.map_err(|_| Error::Disconnected)? }
    )
    .map_err(|err| (None, err))?;

    Ok(())
}

async fn process_requests<A: CustomApi>(
    outstanding_requests: OutstandingRequestMapHandle<A::Request, CustomApiResult<A>>,
    request_receiver: &Receiver<PendingRequest<A::Request, CustomApiResult<A>>>,
    payload_sender: fabruic::Sender<Payload<Request<A::Request>>>,
) -> Result<(), Error> {
    while let Ok(client_request) = request_receiver.recv_async().await {
        let mut outstanding_requests = outstanding_requests.lock().await;
        payload_sender.send(&client_request.request)?;
        outstanding_requests.insert(
            client_request.request.id.expect("all requests require ids"),
            client_request,
        );
    }

    // Return an error to make sure try_join returns.
    Err(Error::Disconnected)
}

pub async fn process<A: CustomApi>(
    outstanding_requests: OutstandingRequestMapHandle<A::Request, CustomApiResult<A>>,
    mut payload_receiver: fabruic::Receiver<Payload<Response<CustomApiResult<A>>>>,
    custom_api_callback: Option<Arc<dyn CustomApiCallback<A>>>,
    #[cfg(feature = "pubsub")] subscribers: SubscriberMap,
) -> Result<(), Error> {
    while let Some(payload) = payload_receiver.next().await {
        let payload = payload?;
        super::process_response_payload(
            payload,
            &outstanding_requests,
            custom_api_callback.as_deref(),
            #[cfg(feature = "pubsub")]
            &subscribers,
        )
        .await;
    }

    Err(Error::Disconnected)
}

async fn connect<A: CustomApi>(
    url: &Url,
    certificate: Option<&Certificate>,
) -> Result<
    (
        fabruic::Connection<()>,
        fabruic::Sender<Payload<Request<A::Request>>>,
        fabruic::Receiver<Payload<Response<CustomApiResult<A>>>>,
    ),
    Error,
> {
    let endpoint = Endpoint::new_client()
        .map_err(|err| Error::Core(bonsaidb_core::Error::Transport(err.to_string())))?;
    let connecting = if let Some(certificate) = certificate {
        endpoint.connect_pinned(url, certificate, None).await?
    } else {
        endpoint.connect(url).await?
    };

    let connection = connecting.accept::<()>().await?;
    let (sender, receiver) = connection.open_stream(&()).await?;

    Ok((connection, sender, receiver))
}
