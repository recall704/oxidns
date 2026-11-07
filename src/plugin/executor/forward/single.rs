// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{info, warn};

use super::is_timeout_error;
use super::metrics::ForwardMetrics;
use super::upstream_protocol;
use crate::core::context::{DnsContext, ResponseOrigin, ResponseOriginKind, TraceDetail};
use crate::infra::error::{DnsError, Result};
use crate::infra::network::upstream::Upstream;
use crate::infra::observability::metrics::{register_metric_source, unregister_metric_source};
use crate::plugin::Plugin;
use crate::plugin::executor::{ExecStep, Executor};

/// Single-upstream DNS forwarder
///
/// Forwards DNS queries to a single configured upstream server.
/// Handles timeouts and logs errors appropriately.
#[allow(unused)]
#[derive(Debug)]
pub(super) struct SingleDnsForwarder {
    /// Plugin identifier
    pub(super) tag: String,

    /// Upstream DNS resolver
    pub(super) upstream: Box<dyn Upstream>,

    /// Whether to stop the executor chain after a successful upstream response.
    pub(super) short_circuit: bool,

    pub(super) metrics: Arc<ForwardMetrics>,
}

#[async_trait]
impl Plugin for SingleDnsForwarder {
    fn tag(&self) -> &str {
        self.tag.as_str()
    }

    async fn init(&mut self, _context: &crate::plugin::PluginInitContext<'_>) -> Result<()> {
        info!("DNS SingleDnsForwarder initialized tag: {}", self.tag);
        register_metric_source(self.metrics.clone())
    }

    async fn destroy(&self) -> Result<()> {
        unregister_metric_source(&self.tag);
        Ok(())
    }
}

#[async_trait]
impl Executor for SingleDnsForwarder {
    #[hotpath::measure]
    async fn execute(&self, context: &mut DnsContext) -> Result<ExecStep> {
        let start_ms = self.metrics.record_query_start();
        let upstream_start = self.metrics.record_upstream_start(0);
        let connection = self.upstream.connection_info();
        let upstream_addr: Arc<str> = Arc::from(connection.raw_addr.as_str());
        let protocol = upstream_protocol(connection);
        match self.upstream.query(context.request.clone()).await {
            Ok(res) => {
                let elapsed_ms =
                    crate::infra::clock::AppClock::elapsed_millis().saturating_sub(upstream_start);
                context.set_response(res);
                context.set_response_origin(
                    ResponseOrigin::new(
                        Arc::<str>::from(self.tag.as_str()),
                        ResponseOriginKind::Forward,
                    )
                    .with_upstream(upstream_addr.clone())
                    .with_elapsed_ms(elapsed_ms),
                );
                let previous = context.trace_enter();
                context.trace_event(
                    "resolver",
                    Some(upstream_addr.as_ref()),
                    "stop",
                    Some(TraceDetail {
                        source: Some(upstream_addr.clone()),
                        note: Some(Arc::from(protocol)),
                        elapsed_ms: Some(elapsed_ms),
                        ..Default::default()
                    }),
                );
                context.trace_leave(previous);
                self.metrics.record_success(start_ms);
                self.metrics.record_upstream_success(0, upstream_start);
            }
            Err(e) => {
                let timeout = is_timeout_error(&e);
                self.metrics.record_error(start_ms, timeout);
                self.metrics
                    .record_upstream_error(0, upstream_start, timeout);
                warn!(
                    "DNS query failed - source: {}, queries: {:?}, id: {}, reason: {}",
                    context.peer_addr(),
                    context.request.questions(),
                    context.request.id(),
                    e
                );
                return Err(DnsError::plugin(format!(
                    "forward plugin '{}' query failed: {}",
                    self.tag, e
                )));
            }
        }
        Ok(self.completion_step())
    }
}

impl SingleDnsForwarder {
    #[inline]
    fn completion_step(&self) -> ExecStep {
        if self.short_circuit {
            ExecStep::Stop
        } else {
            ExecStep::Next
        }
    }
}
