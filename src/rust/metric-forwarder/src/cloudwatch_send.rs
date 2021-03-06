use crate::cloudwatch_logs_parse::Stat;
use crate::error::MetricForwarderError;
use async_trait::async_trait;
use futures::future;
use log::info;
use log::warn;
use rayon::prelude::*;
use rusoto_cloudwatch::PutMetricDataError;
use rusoto_cloudwatch::{CloudWatch, Dimension, MetricDatum, PutMetricDataInput};
use rusoto_core::RusotoError;
use statsd_parser;
use statsd_parser::Metric;
use std::collections::BTreeMap;

pub mod units {
    // strings accepted by CloudWatch MetricDatum.unit
    pub const COUNT: &'static str = "Count";
    pub const MILLIS: &'static str = "Milliseconds";
}

type PutResult = Result<(), RusotoError<PutMetricDataError>>;

#[async_trait]
pub trait CloudWatchPutter {
    // a subset of trait CloudWatch with the 1 function we want
    async fn put_metric_data(&self, input: PutMetricDataInput) -> PutResult;
}

#[async_trait]
impl<T> CloudWatchPutter for T
where
    T: CloudWatch + Sync + Send,
{
    async fn put_metric_data(&self, input: PutMetricDataInput) -> PutResult {
        CloudWatch::put_metric_data(self, input).await
    }
}

pub async fn put_metric_data(
    client: &impl CloudWatchPutter,
    metrics: &[MetricDatum],
    namespace: &str,
) -> Result<(), MetricForwarderError> {
    /*
    Call Cloudwatch to insert metric data. Does batching on our behalf.
    */
    let chunks = metrics.chunks(20).map(|chunk| chunk.to_vec());
    let put_requests = chunks.map(|data: Vec<MetricDatum>| PutMetricDataInput {
        namespace: namespace.to_string(),
        metric_data: data,
    });

    let request_futures = put_requests.map(|input| client.put_metric_data(input));
    let responses: Vec<PutResult> = future::join_all(request_futures).await;

    // TODO: retries

    // bubble up 1 of N failures
    let num_failures = responses.iter().filter(|resp| resp.is_err()).count();
    info!(
        "Sent {} batch-requests to Cloudwatch, of which {} failed",
        responses.len(),
        num_failures
    );
    let first_failure = responses.iter().filter(|resp| resp.is_err()).next();
    match first_failure {
        Some(Err(e)) => Err(MetricForwarderError::PutMetricDataError(e.to_string())),
        _ => Ok(()),
    }
}

/// A nice simplification of our Metric Forwarder is that:
///  since we process things per-log-group,
/// and one log group only has things from one service
/// ---> each execution of this lambda will only be dealing with 1 namespace.
/// We simply assert this invariant in this function.
/// A more robust way would be to group by namespace, but, eh, not needed for us
pub fn get_namespace(stats: &[Stat]) -> Result<String, MetricForwarderError> {
    if let Some(first) = stats.get(0) {
        let expected_namespace = first.service_name.clone();
        let find_different_namespace = stats
            .par_iter()
            .find_any(|s| s.service_name != expected_namespace);
        match find_different_namespace {
            Some(different) => Err(MetricForwarderError::MoreThanOneNamespaceError(
                expected_namespace.to_string(),
                different.service_name.to_string(),
            )),
            None => Ok(expected_namespace),
        }
    } else {
        // I don't expect this to ever happen.
        Err(MetricForwarderError::NoLogsError())
    }
}

pub fn filter_invalid_stats(parsed_stats: Vec<Result<Stat, MetricForwarderError>>) -> Vec<Stat> {
    parsed_stats
        .into_par_iter()
        .filter_map(|stat_res| match stat_res {
            Ok(stat) => Some(stat),
            Err(e) => {
                warn!("Dropped metric: {}", e);
                None
            }
        })
        .collect()
}

pub fn statsd_as_cloudwatch_metric_bulk(parsed_stats: Vec<Stat>) -> Vec<MetricDatum> {
    /*
    Convert the platform-agnostic Stat type to Cloudwatch-specific type.
    */
    parsed_stats
        .into_par_iter()
        .map(statsd_as_cloudwatch_metric)
        .collect()
}

impl From<Stat> for MetricDatum {
    fn from(s: Stat) -> MetricDatum {
        statsd_as_cloudwatch_metric(s)
    }
}

#[derive(Default)]
struct Dimensions(Vec<Dimension>);
/// create Dimensions from statsd Message.tags
impl From<&BTreeMap<String, String>> for Dimensions {
    fn from(source: &BTreeMap<String, String>) -> Dimensions {
        Dimensions(
            source
                .into_iter()
                .map(|(k, v)| Dimension {
                    name: k.to_string(),
                    value: v.to_string(),
                })
                .collect(),
        )
    }
}

fn statsd_as_cloudwatch_metric(stat: Stat) -> MetricDatum {
    let (unit, value, _sample_rate) = match stat.msg.metric {
        // Yes, gauge and counter are - for our purposes - basically both Count
        Metric::Gauge(g) => (units::COUNT, g.value, g.sample_rate),
        Metric::Counter(c) => (units::COUNT, c.value, c.sample_rate),
        Metric::Histogram(h) => (units::MILLIS, h.value, h.sample_rate),
        _ => panic!("How the heck did you get an unsupported metric type in here?"),
    };
    let Dimensions(dims) = stat
        .msg
        .tags
        .as_ref()
        .map(|tags| tags.into())
        .unwrap_or_default();
    // AWS doesn't like sending it an empty list
    let dims_option = match dims.is_empty() {
        true => None,
        false => Some(dims),
    };

    let datum = MetricDatum {
        metric_name: stat.msg.name.to_string(),
        timestamp: stat.timestamp.to_string().into(),
        unit: unit.to_string().into(),
        value: value.into(),
        // TODO seems like cloudwatch has no concept of sample rate, lol
        // many of the following are useful for batching:
        // e.g. counts: [1, 5] + values: [1.0, 2.0] means that
        // 1.0 was observed 1x && 2.0 was observed 5x
        counts: None,
        values: None,
        dimensions: dims_option,
        statistic_values: None,
        storage_resolution: None,
    };
    datum
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE_NAME: &'static str = "cool_service";

    #[test]
    fn test_convert_one_stat_into_datum() {
        let ts = "im timestamp".to_string();
        let name = "im a name".to_string();
        let service_name = SERVICE_NAME.into();
        let counter = statsd_parser::Counter {
            value: 12.3,
            sample_rate: Some(0.5),
        };
        let stat = Stat {
            timestamp: ts.clone(),
            msg: statsd_parser::Message {
                name: name.clone(),
                tags: None,
                metric: statsd_parser::Metric::Counter(counter),
            },
            service_name: service_name,
        };

        let datum: MetricDatum = stat.into();
        assert_eq!(&datum.metric_name, &name);
        assert_eq!(&datum.timestamp.expect(""), &ts);
        assert_eq!(datum.value.expect(""), 12.3);
        assert_eq!(datum.unit.expect(""), units::COUNT);
        assert_eq!(datum.dimensions, None);
    }

    #[test]
    fn test_convert_one_stat_with_tags_into_datum() {
        let stat = some_stat_with_tags();
        let datum: MetricDatum = stat.into();
        assert_eq!(
            datum.dimensions.expect(""),
            vec![
                Dimension {
                    name: "tag1".into(),
                    value: "val1".into()
                },
                Dimension {
                    name: "tag2".into(),
                    value: "val2".into()
                },
            ]
        );
    }

    pub struct MockCloudwatchClient {
        response_fn: fn(PutMetricDataInput) -> PutResult,
    }
    impl MockCloudwatchClient {
        fn return_ok(_input: PutMetricDataInput) -> PutResult {
            Ok(())
        }

        fn return_an_err(_input: PutMetricDataInput) -> PutResult {
            Err(RusotoError::Service(
                PutMetricDataError::InternalServiceFault("ya goofed".to_string()),
            ))
        }
    }

    #[async_trait]
    impl CloudWatchPutter for MockCloudwatchClient {
        async fn put_metric_data(&self, input: PutMetricDataInput) -> PutResult {
            return (self.response_fn)(input);
        }
    }

    fn some_stat() -> Stat {
        Stat {
            timestamp: "ts".into(),
            msg: statsd_parser::Message {
                name: "msg".into(),
                metric: statsd_parser::Metric::Counter(statsd_parser::Counter {
                    value: 123.45,
                    sample_rate: None,
                }),
                tags: None,
            },
            service_name: SERVICE_NAME.into(),
        }
    }

    fn some_stat_with_different_service_name() -> Stat {
        let mut stat = some_stat();
        stat.service_name = "another_service".to_string();
        stat
    }

    fn some_stat_with_tags() -> Stat {
        let mut tags = BTreeMap::<String, String>::new();
        tags.insert("tag2".into(), "val2".into());
        tags.insert("tag1".into(), "val1".into());
        // note - .iter() sorts these by key, so tag1 will show up first!
        Stat {
            timestamp: "ts".into(),
            msg: statsd_parser::Message {
                name: "msg".into(),
                metric: statsd_parser::Metric::Counter(statsd_parser::Counter {
                    value: 123.45,
                    sample_rate: None,
                }),
                tags: tags.into(),
            },
            service_name: SERVICE_NAME.into(),
        }
    }

    #[test]
    fn test_get_namespace_different_service_names() {
        let stats = vec![some_stat(), some_stat_with_different_service_name()];

        let result = get_namespace(&stats);
        match result {
            Ok(_) => panic!("shouldn't get anything here"),
            Err(e) => assert!(e
                .to_string()
                .contains("Expected cool_service, found another_service")),
        }
    }

    #[test]
    fn test_get_namespace_same_service_names() -> Result<(), MetricForwarderError> {
        let stats = vec![some_stat(), some_stat(), some_stat()];

        let result = get_namespace(&stats)?;
        assert_eq!(result, SERVICE_NAME);
        Ok(())
    }

    #[tokio::test]
    async fn test_put_metric_data_client_ok() {
        let cw_client = MockCloudwatchClient {
            response_fn: MockCloudwatchClient::return_ok,
        };
        let data = vec![some_stat().into(), some_stat().into()];
        let result = put_metric_data(&cw_client, &data, SERVICE_NAME).await;
        assert_eq!(result, Ok(()))
    }

    #[tokio::test]
    async fn test_put_metric_data_client_err() -> Result<(), ()> {
        let cw_client = MockCloudwatchClient {
            response_fn: MockCloudwatchClient::return_an_err,
        };
        let data = vec![some_stat().into(), some_stat().into()];
        let result = put_metric_data(&cw_client, &data, SERVICE_NAME).await;
        match result {
            Err(MetricForwarderError::PutMetricDataError(_)) => Ok(()),
            _ => Err(()),
        }
    }

    // I had to use a 'prod test' to figure out why a line wasn't being parsed correctly.
    // I'll leave it here in case that ever needs to happen again in the future.
    /*
    use rusoto_core::region::Region;
    use rusoto_cloudwatch::CloudWatchClient;
    #[tokio::test]
    async fn test_put_metric_data_prod() -> Result<(), MetricForwarderError> {
        let input = "MONITORING|sysmon_subgraph_generator|2020-09-21T23:16:47.868Z|sysmon-generator-completion:1|g|#status:success";
        let as_stat = crate::cloudwatch_logs_parse::parse_log(input)?;
        let cw_client = CloudWatchClient::new(Region::UsWest2);
        let data = vec![as_stat.into()];
        println!("{:?}", &data);
        let result = put_metric_data(&cw_client, &data, SERVICE_NAME).await;
        assert_eq!(result, Ok(()));
        Ok(())
    }
    */
}
