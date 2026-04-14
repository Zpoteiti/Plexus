# Issues — plexus-gateway

Open, deferred, and closed issues for the gateway crate. Updated by `/wrap-up` at the end of each session.

## Open
<!-- Active issues that need attention this session or next -->

## Deferred
<!-- Acknowledged but intentionally postponed — include context and date -->
- [ ] Gateway uses transient WS connection UUID as chat_id in InboundEvent instead of stable session_id (context: agent sees a different chat_id after every browser reconnect, breaking cron delivery to gateway channel; fix agreed but deferred — gateway must index browser connections by session_id and pass session_id as chat_id, 2026-04-14)

## Closed
<!-- Resolved issues — keep for historical context -->
