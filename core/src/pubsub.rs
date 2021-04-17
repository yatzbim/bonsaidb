use async_trait::async_trait;
use circulate::{Relay, Subscriber};
use serde::Serialize;

use crate::Error;

/// Publishes and Subscribes to messages on topics.
#[async_trait]
pub trait PubSub {
    /// Create a new [`Subscriber`] for this relay.
    async fn create_subscriber(&self) -> Result<Subscriber, Error>;
    /// Publishes a `payload` to all subscribers of `topic`.
    async fn publish<S: Into<String> + Send, P: Serialize + Sync>(
        &self,
        topic: S,
        payload: &P,
    ) -> Result<(), Error>;
}

#[async_trait]
impl PubSub for Relay {
    async fn create_subscriber(&self) -> Result<Subscriber, Error> {
        Ok(self.create_subscriber().await)
    }

    async fn publish<S: Into<String> + Send, P: Serialize + Sync>(
        &self,
        topic: S,
        payload: &P,
    ) -> Result<(), Error> {
        self.publish(topic, payload).await?;
        Ok(())
    }
}

/// Expands into a suite of pubsub unit tests using the passed type as the test harness.
#[cfg(any(test, feature = "test-util"))]
#[cfg_attr(feature = "test-util", macro_export)]
macro_rules! define_pubsub_test_suite {
    ($harness:ident) => {
        #[cfg(test)]
        use $crate::pubsub::PubSub;

        #[tokio::test]
        async fn simple_pubsub_test() -> anyhow::Result<()> {
            let harness = $harness::new($crate::test_util::HarnessTest::PubSubSimple).await?;
            let pubsub = harness.connect().await?;
            let subscriber = PubSub::create_subscriber(&pubsub).await?;
            subscriber.subscribe_to("mytopic").await;
            pubsub.publish("mytopic", &String::from("test")).await?;
            let receiver = subscriber.receiver().clone();
            let message = receiver.recv_async().await.expect("No message received");
            assert_eq!(message.topic, "mytopic");
            assert_eq!(message.payload::<String>()?, "test");
            // The message should only be received once.
            assert!(matches!(
                tokio::task::spawn_blocking(
                    move || receiver.recv_timeout(std::time::Duration::from_millis(100))
                )
                .await,
                Ok(Err(_))
            ));
            Ok(())
        }

        #[tokio::test]
        async fn multiple_subscribers_test() -> anyhow::Result<()> {
            let harness =
                $harness::new($crate::test_util::HarnessTest::PubSubMultipleSubscribers).await?;
            let pubsub = harness.connect().await?;
            let subscriber_a = PubSub::create_subscriber(&pubsub).await?;
            let subscriber_ab = PubSub::create_subscriber(&pubsub).await?;
            subscriber_a.subscribe_to("a").await;
            subscriber_ab.subscribe_to("a").await;
            subscriber_ab.subscribe_to("b").await;

            pubsub.publish("a", &String::from("a1")).await?;
            pubsub.publish("b", &String::from("b1")).await?;
            pubsub.publish("a", &String::from("a2")).await?;

            // Check subscriber_a for a1 and a2.
            let message = subscriber_a.receiver().recv_async().await?;
            assert_eq!(message.payload::<String>()?, "a1");
            let message = subscriber_a.receiver().recv_async().await?;
            assert_eq!(message.payload::<String>()?, "a2");

            let message = subscriber_ab.receiver().recv_async().await?;
            assert_eq!(message.payload::<String>()?, "a1");
            let message = subscriber_ab.receiver().recv_async().await?;
            assert_eq!(message.payload::<String>()?, "b1");
            let message = subscriber_ab.receiver().recv_async().await?;
            assert_eq!(message.payload::<String>()?, "a2");

            Ok(())
        }

        #[tokio::test]
        async fn unsubscribe_test() -> anyhow::Result<()> {
            let harness = $harness::new($crate::test_util::HarnessTest::PubSubUnsubscribe).await?;
            let pubsub = harness.connect().await?;
            let subscriber = PubSub::create_subscriber(&pubsub).await?;
            subscriber.subscribe_to("a").await;

            pubsub.publish("a", &String::from("a1")).await?;
            subscriber.unsubscribe_from("a").await;
            pubsub.publish("a", &String::from("a2")).await?;
            subscriber.subscribe_to("a").await;
            pubsub.publish("a", &String::from("a3")).await?;

            // Check subscriber_a for a1 and a2.
            let message = subscriber.receiver().recv_async().await?;
            assert_eq!(message.payload::<String>()?, "a1");
            let message = subscriber.receiver().recv_async().await?;
            assert_eq!(message.payload::<String>()?, "a3");

            Ok(())
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::HarnessTest;

    struct Harness {
        relay: Relay,
    }

    impl Harness {
        async fn new(_: HarnessTest) -> Result<Self, Error> {
            Ok(Self {
                relay: Relay::default(),
            })
        }

        async fn connect(&self) -> Result<Relay, Error> {
            Ok(self.relay.clone())
        }
    }

    define_pubsub_test_suite!(Harness);
}
