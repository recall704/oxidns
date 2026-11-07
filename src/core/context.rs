// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! DNS request/response context management.

use std::any::Any;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use ahash::{AHashMap, AHashSet};

use crate::proto::Message;

/// Typed metadata attached to the request by the inbound server layer.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct RequestMeta {
    /// SNI or host-like server identifier carried by the server layer.
    pub server_name: Option<Arc<str>>,
    /// URL path carried by HTTP-based server layers.
    pub url_path: Option<Arc<str>>,
}

/// Metadata carried by the inbound transport layer.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IngressContext {
    peer_addr: SocketAddr,
    request_meta: RequestMeta,
    server_tag: Option<Arc<str>>,
    protocol: Option<Arc<str>>,
    listen: Option<SocketAddr>,
    entry: Option<Arc<str>>,
}

impl Default for IngressContext {
    fn default() -> Self {
        Self {
            peer_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            request_meta: RequestMeta::default(),
            server_tag: None,
            protocol: None,
            listen: None,
            entry: None,
        }
    }
}

impl IngressContext {
    #[inline]
    pub fn new(peer_addr: SocketAddr) -> Self {
        Self {
            peer_addr,
            request_meta: RequestMeta::default(),
            server_tag: None,
            protocol: None,
            listen: None,
            entry: None,
        }
    }

    #[inline]
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    #[inline]
    pub fn set_peer_addr(&mut self, peer_addr: SocketAddr) {
        self.peer_addr = peer_addr;
    }

    #[inline]
    pub fn server_tag(&self) -> Option<&str> {
        self.server_tag.as_deref()
    }

    #[inline]
    pub fn protocol(&self) -> Option<&str> {
        self.protocol.as_deref()
    }

    #[inline]
    pub fn listen(&self) -> Option<SocketAddr> {
        self.listen
    }

    #[inline]
    pub fn entry(&self) -> Option<&str> {
        self.entry.as_deref()
    }

    /// Attach the inbound server identity captured by `RequestHandle`.
    pub fn set_server_identity(
        &mut self,
        server_tag: Arc<str>,
        protocol: Arc<str>,
        listen: Option<SocketAddr>,
        entry: Arc<str>,
    ) {
        self.server_tag = Some(server_tag);
        self.protocol = Some(protocol);
        self.listen = listen;
        self.entry = Some(entry);
    }

    #[inline]
    pub fn request_meta(&self) -> &RequestMeta {
        &self.request_meta
    }

    #[inline]
    pub fn set_request_meta(&mut self, meta: RequestMeta) {
        self.request_meta = meta;
    }

    #[inline]
    pub fn server_name(&self) -> Option<&str> {
        self.request_meta.server_name.as_deref()
    }

    #[inline]
    pub fn url_path(&self) -> Option<&str> {
        self.request_meta.url_path.as_deref()
    }
}

/// Runtime-only mutable execution state.
#[derive(Debug)]
pub struct RuntimeContext {
    marks: AHashSet<u32>,
    extensions: AHashMap<String, Box<dyn Any + Send + Sync>>,
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self {
            marks: AHashSet::new(),
            extensions: AHashMap::new(),
        }
    }
}

impl RuntimeContext {
    #[inline]
    pub fn marks(&self) -> &AHashSet<u32> {
        &self.marks
    }

    #[inline]
    pub fn marks_mut(&mut self) -> &mut AHashSet<u32> {
        &mut self.marks
    }

    pub fn set_attr<T>(&mut self, name: impl Into<String>, value: T)
    where
        T: Send + Sync + 'static,
    {
        self.extensions.insert(name.into(), Box::new(value));
    }

    pub fn get_attr<T>(&self, name: &str) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.extensions
            .get(name)
            .and_then(|value| value.downcast_ref())
    }

    pub fn contains_attr(&self, name: &str) -> bool {
        self.extensions.contains_key(name)
    }

    pub fn remove_attr<T>(&mut self, name: &str) -> Option<T>
    where
        T: Send + Sync + 'static,
    {
        self.extensions
            .remove(name)
            .and_then(|value| value.downcast::<T>().ok())
            .map(|boxed| *boxed)
    }
}

/// Optional per-event detail used by the request path view.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct TraceDetail {
    /// Value observed by a matcher, such as the query name or client address.
    pub observed: Option<Arc<str>>,
    /// Jump/goto target sequence tag.
    pub target: Option<Arc<str>>,
    /// Responder or resolver source, such as an upstream address.
    pub source: Option<Arc<str>>,
    /// Small human-readable annotation (rcode, hit/miss, fallback reason).
    pub note: Option<Arc<str>>,
    /// Latency of the nested exchange this event describes.
    pub elapsed_ms: Option<u64>,
}

/// How the response was produced.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ResponseOriginKind {
    Executor,
    Cache,
    Hosts,
    Forward,
    Synthetic,
    ServerFallback,
    ServerError,
}

impl ResponseOriginKind {
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Executor => "executor",
            Self::Cache => "cache",
            Self::Hosts => "hosts",
            Self::Forward => "forward",
            Self::Synthetic => "synthetic",
            Self::ServerFallback => "server_fallback",
            Self::ServerError => "server_error",
        }
    }
}

/// The executor and external source that produced the response.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResponseOrigin {
    pub executor_tag: Arc<str>,
    pub kind: ResponseOriginKind,
    pub upstream: Option<Arc<str>>,
    pub elapsed_ms: Option<u64>,
}

impl ResponseOrigin {
    #[inline]
    pub fn new(executor_tag: impl Into<Arc<str>>, kind: ResponseOriginKind) -> Self {
        Self {
            executor_tag: executor_tag.into(),
            kind,
            upstream: None,
            elapsed_ms: None,
        }
    }

    #[inline]
    pub fn with_upstream(mut self, upstream: impl Into<Arc<str>>) -> Self {
        self.upstream = Some(upstream.into());
        self
    }

    #[inline]
    pub fn with_elapsed_ms(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = Some(elapsed_ms);
        self
    }
}

/// One structured execution-path event captured from the sequence runtime.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExecutionPathEvent {
    pub sequence_tag: String,
    pub node_index: Option<usize>,
    pub kind: String,
    pub tag: Option<String>,
    pub outcome: String,
    /// Visual tree depth maintained by `ExecutionPath` scopes.
    pub depth: u16,
    /// Microseconds since the path started recording.
    pub at_us: u64,
    pub detail: Option<Arc<TraceDetail>>,
}

impl ExecutionPathEvent {
    #[inline]
    pub fn new(
        sequence_tag: impl Into<String>,
        node_index: Option<usize>,
        kind: impl Into<String>,
        tag: Option<impl Into<String>>,
        outcome: impl Into<String>,
    ) -> Self {
        Self {
            sequence_tag: sequence_tag.into(),
            node_index,
            kind: kind.into(),
            tag: tag.map(Into::into),
            outcome: outcome.into(),
            depth: 0,
            at_us: 0,
            detail: None,
        }
    }

    #[inline]
    pub fn with_detail(mut self, detail: TraceDetail) -> Self {
        self.detail = Some(Arc::new(detail));
        self
    }
}

/// Request-local execution path recording state.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct ExecutionPath {
    enabled: bool,
    events: Vec<Arc<ExecutionPathEvent>>,
    depth: u16,
    sequence_tags: Vec<Arc<str>>,
    started: Option<Instant>,
}

impl ExecutionPath {
    #[inline]
    pub fn enable(&mut self) {
        if !self.enabled {
            self.enabled = true;
            self.started = Some(Instant::now());
        }
    }

    #[inline]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Enter a non-sequence scope and return the previous depth.
    #[inline]
    pub fn enter_scope(&mut self) -> u16 {
        let previous = self.depth;
        self.depth = self.depth.saturating_add(1);
        previous
    }

    /// Restore the depth captured by [`Self::enter_scope`].
    #[inline]
    pub fn leave_scope(&mut self, previous: u16) {
        self.depth = previous;
    }

    /// Enter a sequence invocation scope and return the previous depth.
    #[inline]
    pub fn enter_sequence(&mut self, tag: &Arc<str>) -> u16 {
        if self.enabled {
            self.sequence_tags.push(tag.clone());
        }
        self.enter_scope()
    }

    /// Leave the current sequence invocation.
    #[inline]
    pub fn leave_sequence(&mut self, previous: u16) {
        if self.enabled {
            self.sequence_tags.pop();
        }
        self.leave_scope(previous);
    }

    #[inline]
    pub fn current_sequence_tag(&self) -> Option<&str> {
        self.sequence_tags.last().map(AsRef::as_ref)
    }

    #[inline]
    pub fn push(&mut self, mut event: ExecutionPathEvent) {
        if !self.enabled {
            return;
        }
        event.depth = self.depth;
        event.at_us = self
            .started
            .map(|started| started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        if event.sequence_tag.is_empty()
            && let Some(tag) = self.sequence_tags.last()
        {
            event.sequence_tag = tag.to_string();
        }
        self.events.push(Arc::new(event));
    }

    #[inline]
    pub fn events(&self) -> &[Arc<ExecutionPathEvent>] {
        &self.events
    }

    #[inline]
    pub fn events_from(&self, start: usize) -> &[Arc<ExecutionPathEvent>] {
        self.events.get(start..).unwrap_or(&[])
    }
}

/// Context object for a DNS request/response lifecycle.
pub struct DnsContext {
    pub ingress: IngressContext,
    pub request: Message,
    pub response: Option<Message>,
    pub execution_path: ExecutionPath,
    pub runtime: RuntimeContext,
    response_origin: Option<ResponseOrigin>,
}

impl DnsContext {
    #[inline]
    pub fn new(peer_addr: SocketAddr, request: Message) -> Self {
        Self {
            ingress: IngressContext::new(peer_addr),
            request,
            response: None,
            execution_path: ExecutionPath::default(),
            runtime: RuntimeContext::default(),
            response_origin: None,
        }
    }

    #[inline]
    pub fn peer_addr(&self) -> SocketAddr {
        self.ingress.peer_addr()
    }

    #[inline]
    pub fn set_peer_addr(&mut self, peer_addr: SocketAddr) {
        self.ingress.set_peer_addr(peer_addr);
    }

    #[inline]
    pub fn set_request_meta(&mut self, meta: RequestMeta) {
        self.ingress.set_request_meta(meta);
    }

    #[inline]
    pub fn request_meta(&self) -> &RequestMeta {
        self.ingress.request_meta()
    }

    #[inline]
    pub fn server_name(&self) -> Option<&str> {
        self.ingress.server_name()
    }

    #[inline]
    pub fn url_path(&self) -> Option<&str> {
        self.ingress.url_path()
    }

    #[inline]
    pub fn server_tag(&self) -> Option<&str> {
        self.ingress.server_tag()
    }

    #[inline]
    pub fn protocol(&self) -> Option<&str> {
        self.ingress.protocol()
    }

    #[inline]
    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.ingress.listen()
    }

    #[inline]
    pub fn entry_tag(&self) -> Option<&str> {
        self.ingress.entry()
    }

    #[inline]
    pub fn set_server_identity(
        &mut self,
        server_tag: Arc<str>,
        protocol: Arc<str>,
        listen: Option<SocketAddr>,
        entry: Arc<str>,
    ) {
        self.ingress
            .set_server_identity(server_tag, protocol, listen, entry);
    }

    #[inline]
    pub fn request(&self) -> &Message {
        &self.request
    }

    #[inline]
    pub fn request_mut(&mut self) -> &mut Message {
        &mut self.request
    }

    #[inline]
    pub fn replace_request(&mut self, request: Message) {
        self.request = request;
    }

    #[inline]
    pub fn response(&self) -> Option<&Message> {
        self.response.as_ref()
    }

    #[inline]
    pub fn response_mut(&mut self) -> Option<&mut Message> {
        self.response.as_mut()
    }

    #[inline]
    pub fn set_response(&mut self, response: Message) {
        self.response = Some(response);
    }

    #[inline]
    pub fn clear_response(&mut self) {
        self.response = None;
    }

    #[inline]
    pub fn take_response(&mut self) -> Option<Message> {
        self.response.take()
    }

    #[inline]
    pub fn response_origin(&self) -> Option<&ResponseOrigin> {
        self.response_origin.as_ref()
    }

    #[inline]
    pub fn set_response_origin(&mut self, origin: ResponseOrigin) {
        self.response_origin = Some(origin);
    }

    /// Attribute a newly produced response to `executor_tag` unless a more
    /// specific producer already claimed it.
    #[inline]
    pub fn ensure_response_origin(&mut self, executor_tag: &str) {
        if self.response_origin.is_none() {
            self.response_origin = Some(ResponseOrigin::new(
                Arc::<str>::from(executor_tag),
                ResponseOriginKind::Executor,
            ));
        }
    }

    /// Enter a nested trace scope and return the previous depth.
    #[inline]
    pub fn trace_enter(&mut self) -> u16 {
        self.execution_path.enter_scope()
    }

    /// Restore a depth returned by [`Self::trace_enter`].
    #[inline]
    pub fn trace_leave(&mut self, previous: u16) {
        self.execution_path.leave_scope(previous);
    }

    /// Enter a sequence invocation scope and return the previous depth.
    #[inline]
    pub fn trace_enter_sequence(&mut self, tag: &Arc<str>) -> u16 {
        self.execution_path.enter_sequence(tag)
    }

    /// Leave a sequence invocation scope.
    #[inline]
    pub fn trace_leave_sequence(&mut self, previous: u16) {
        self.execution_path.leave_sequence(previous);
    }

    /// Record a trace event emitted by a plugin rather than the sequence
    /// runtime. No work is performed when path recording is disabled.
    pub fn trace_event(
        &mut self,
        kind: &str,
        tag: Option<&str>,
        outcome: &str,
        detail: Option<TraceDetail>,
    ) {
        if !self.execution_path_enabled() {
            return;
        }
        let mut event = ExecutionPathEvent::new("", None, kind, tag, outcome);
        if let Some(detail) = detail {
            event = event.with_detail(detail);
        }
        self.push_execution_path_event(event);
    }

    #[inline]
    pub fn marks(&self) -> &AHashSet<u32> {
        self.runtime.marks()
    }

    #[inline]
    pub fn marks_mut(&mut self) -> &mut AHashSet<u32> {
        self.runtime.marks_mut()
    }

    #[inline]
    pub fn contains_attr(&self, name: &str) -> bool {
        self.runtime.contains_attr(name)
    }

    #[inline]
    pub fn set_attr<T>(&mut self, name: impl Into<String>, value: T)
    where
        T: Send + Sync + 'static,
    {
        self.runtime.set_attr(name, value);
    }

    #[inline]
    pub fn get_attr<T>(&self, name: &str) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.runtime.get_attr(name)
    }

    #[inline]
    pub fn take_attr<T>(&mut self, name: &str) -> Option<T>
    where
        T: Send + Sync + 'static,
    {
        self.runtime.remove_attr(name)
    }

    #[inline]
    pub fn enable_execution_path(&mut self) {
        self.execution_path.enable();
    }

    #[inline]
    pub fn execution_path_enabled(&self) -> bool {
        self.execution_path.enabled()
    }

    #[inline]
    pub fn execution_path_len(&self) -> usize {
        self.execution_path.len()
    }

    #[inline]
    pub fn execution_path_events(&self) -> &[Arc<ExecutionPathEvent>] {
        self.execution_path.events()
    }

    #[inline]
    pub fn push_execution_path_event(&mut self, event: ExecutionPathEvent) {
        self.execution_path.push(event);
    }

    pub fn copy_for_subquery(&self) -> DnsContext {
        DnsContext {
            ingress: self.ingress.clone(),
            request: self.request.clone(),
            response: self.response.clone(),
            execution_path: self.execution_path.clone(),
            runtime: RuntimeContext {
                marks: self.runtime.marks.clone(),
                extensions: AHashMap::new(),
            },
            response_origin: self.response_origin.clone(),
        }
    }

    pub fn apply_subquery_result(&mut self, sub_ctx: DnsContext) {
        self.ingress = sub_ctx.ingress;
        self.request = sub_ctx.request;
        self.response = sub_ctx.response;
        self.execution_path = sub_ctx.execution_path;
        self.runtime.marks = sub_ctx.runtime.marks;
        self.runtime.extensions = sub_ctx.runtime.extensions;
        self.response_origin = sub_ctx.response_origin;
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::*;
    use crate::proto::rdata::A;
    use crate::proto::{DNSClass, Message, Name, Question, RData, Record, RecordType};

    fn make_context() -> DnsContext {
        let mut request = Message::new();
        request.add_question(Question::new(
            Name::from_ascii("WWW.Example.COM.").unwrap(),
            RecordType::A,
            DNSClass::IN,
        ));
        DnsContext::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 5300)), request)
    }

    #[test]
    fn test_request_meta_is_typed() {
        let mut ctx = make_context();
        ctx.set_request_meta(RequestMeta {
            server_name: Some(Arc::from("dns.example.com")),
            url_path: Some(Arc::from("/dns-query")),
        });

        assert_eq!(ctx.server_name(), Some("dns.example.com"));
        assert_eq!(ctx.url_path(), Some("/dns-query"));
    }

    #[test]
    fn test_request_helpers_replace_message() {
        let mut ctx = make_context();
        let packet = vec![
            0x12, 0x34, 0x01, 0x10, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, b'a',
            b'p', b'i', 0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'c', b'o', b'm',
            0x00, 0x00, 0x1C, 0x00, 0x01,
        ];

        ctx.replace_request(Message::from_bytes(&packet).expect("packet should parse"));
        let question = ctx.request.first_question().expect("question should exist");
        assert_eq!(question.name().normalized(), "api.example.com");
        assert_eq!(question.qtype(), RecordType::AAAA);
        assert!(ctx.request.checking_disabled());
    }

    #[test]
    fn test_response_is_mutated_directly() {
        let mut ctx = make_context();
        let mut response = Message::new();
        response.set_message_type(crate::proto::MessageType::Response);
        response.add_question(Question::new(
            Name::from_ascii("www.example.com.").unwrap(),
            RecordType::A,
            DNSClass::IN,
        ));
        response.add_answer(Record::from_rdata(
            Name::from_ascii("www.example.com.").unwrap(),
            60,
            RData::A(A(Ipv4Addr::new(192, 0, 2, 1))),
        ));
        ctx.set_response(response);

        assert!(
            ctx.response
                .as_ref()
                .unwrap()
                .has_answer_ip(|ip| ip == IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)))
        );

        ctx.response
            .as_mut()
            .unwrap()
            .add_answer(Record::from_rdata(
                Name::from_ascii("www.example.com.").unwrap(),
                60,
                RData::A(A(Ipv4Addr::new(198, 51, 100, 2))),
            ));

        assert!(
            ctx.response
                .as_ref()
                .unwrap()
                .has_answer_ip(|ip| ip == IpAddr::V4(Ipv4Addr::new(198, 51, 100, 2)))
        );
    }

    #[test]
    fn test_execution_path_is_opt_in() {
        let mut ctx = make_context();
        ctx.push_execution_path_event(ExecutionPathEvent::new(
            "main",
            Some(0),
            "matcher",
            Some("qname"),
            "matched",
        ));
        assert!(ctx.execution_path_events().is_empty());

        ctx.enable_execution_path();
        ctx.push_execution_path_event(ExecutionPathEvent::new(
            "main",
            Some(0),
            "matcher",
            Some("qname"),
            "matched",
        ));
        assert_eq!(ctx.execution_path_len(), 1);
    }

    #[test]
    fn test_execution_path_subquery_copy_and_apply() {
        let mut ctx = make_context();
        ctx.enable_execution_path();
        ctx.push_execution_path_event(ExecutionPathEvent::new(
            "main",
            Some(0),
            "executor",
            Some("cache"),
            "entered",
        ));

        let mut sub_ctx = ctx.copy_for_subquery();
        sub_ctx.push_execution_path_event(ExecutionPathEvent::new(
            "main",
            Some(1),
            "executor",
            Some("forward"),
            "next",
        ));
        ctx.apply_subquery_result(sub_ctx);

        assert_eq!(ctx.execution_path_len(), 2);
        assert_eq!(
            ctx.execution_path.events_from(1)[0].tag.as_deref(),
            Some("forward")
        );
    }

    #[test]
    fn test_execution_path_records_depth_timing_and_detail() {
        let mut ctx = make_context();
        ctx.enable_execution_path();
        let sequence_depth = ctx.trace_enter_sequence(&Arc::from("main"));
        ctx.push_execution_path_event(ExecutionPathEvent::new(
            "main",
            Some(0),
            "matcher",
            Some("qname"),
            "matched",
        ));
        let rule_depth = ctx.trace_enter();
        ctx.push_execution_path_event(
            ExecutionPathEvent::new("main", Some(0), "executor", Some("hosts"), "stop")
                .with_detail(TraceDetail {
                    observed: Some(Arc::from("api.example.com.")),
                    source: Some(Arc::from("hosts")),
                    ..Default::default()
                }),
        );
        ctx.trace_leave(rule_depth);
        ctx.trace_leave_sequence(sequence_depth);

        let events = ctx.execution_path_events();
        assert_eq!(events[0].depth, 1);
        assert_eq!(events[1].depth, 2);
        assert!(events[1].at_us >= events[0].at_us);
        assert_eq!(
            events[1]
                .detail
                .as_ref()
                .and_then(|detail| detail.observed.as_deref()),
            Some("api.example.com.")
        );
    }

    #[test]
    fn test_trace_event_uses_current_sequence_tag() {
        let mut ctx = make_context();
        ctx.enable_execution_path();
        let sequence_depth = ctx.trace_enter_sequence(&Arc::from("main"));
        ctx.trace_event(
            "resolver",
            Some("223.5.5.5:53"),
            "stop",
            Some(TraceDetail {
                elapsed_ms: Some(18),
                ..Default::default()
            }),
        );
        ctx.trace_leave_sequence(sequence_depth);

        let event = &ctx.execution_path_events()[0];
        assert_eq!(event.sequence_tag, "main");
        assert_eq!(event.kind, "resolver");
        assert_eq!(event.depth, 1);
        assert_eq!(
            event.detail.as_ref().and_then(|detail| detail.elapsed_ms),
            Some(18)
        );
    }
}
