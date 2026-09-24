# HalluScribe Business Messaging Ingestion Plan

## Goal
Extend HalluScribe so it can ingest business conversations from consumer messaging sources and make them searchable through the existing archive/MCP flow.

Primary use case:

> "A client a few months ago ordered Calacatta Light. Find the conversation and tell me what happened."

The agent should be able to locate the exact historical conversation, inspect the raw transcript, resolve the client identity from contacts when possible, and summarize the order history with dates and relevant details.

## Scope

### Phase 1 sources
1. **Viber Desktop on Windows**
   - Read the local Viber SQLite database when available.
   - Preserve thread/contact/message metadata.
   - Prefer direct local DB ingestion over manual CSV export.

2. **Apple Messages from a local iPhone backup on Windows**
   - Support backups created by Apple Devices / iTunes.
   - Resolve the logical Messages database through `Manifest.db` rather than assuming a visible `sms.db` filename.
   - Expected logical path: `HomeDomain/Library/SMS/sms.db`.
   - Resolve message attachments from the backup where practical.

3. **iPhone Contacts / Phone Book from the same local backup**
   - Expected logical path: `HomeDomain/Library/AddressBook/AddressBook.sqlitedb`.
   - Optionally support `AddressBookImages.sqlitedb` for contact photos later.
   - Use contacts to resolve message handles/phone numbers to human-readable client identities.

### Possible later sources
- WhatsApp / WhatsApp Business local exports or backups where a reliable read-only path exists.
- macOS Messages via `~/Library/Messages/chat.db`.
- Additional business messaging apps using the same adapter interface.

## Design Principle
Do not create a separate search system for business messages. Normalize them into HalluScribe's existing archive model so the current MCP tools remain useful:

- `search_sessions`
- `read_session`
- `search_raw_transcripts`
- `read_raw_session`
- `get_profile`
- `get_digest`

For exact business facts such as product names, slab codes, quantities, dimensions, prices, dates, and phone numbers, raw transcript search is the authoritative retrieval path. Summaries are a convenience layer, not the source of truth.

## Target Architecture

```text
Viber Desktop SQLite
        |
        v
   Viber importer
        |
        +-----------------------+
                                |
iPhone local backup             |
  Manifest.db                   |
  sms.db                        |
  AddressBook.sqlitedb          |
  attachments                   |
        |                       |
        v                       |
 Apple backup importer          |
        |                       |
        +----------+------------+
                   |
                   v
        Normalized message thread
                   |
                   v
        HalluScribe preprocessing
                   |
          +--------+---------+
          |                  |
          v                  v
   Raw preserved copy   Model summary
          |                  |
          +--------+---------+
                   |
                   v
            Archive + index
                   |
                   v
              MCP / Forge
```

## Normalized Conversation Model
The adapters should converge on one internal representation before archive generation.

Suggested fields:

```text
source                 viber | apple_messages | sms | rcs | ...
conversation_id        source-stable thread id
conversation_name      display/thread title
participants[]         stable participant records
started_at             first message timestamp
ended_at               last message timestamp
message_count          count
messages[]              ordered messages
```

Suggested participant fields:

```text
participant_id
name
company
phone_numbers[]
email_addresses[]
source_contact_id
```

Suggested message fields:

```text
message_id
timestamp
direction              incoming | outgoing | unknown
sender_participant_id
text
attachment_refs[]
reply_to_message_id     when available
status                  optional source-specific delivery state
```

Do not force source-specific data into the common model if it has no equivalent. Preserve optional source metadata separately.

## Contact Resolution
Contacts should be treated as a first-class enrichment source, not just a UI convenience.

Resolution order should be deterministic:

1. Exact normalized phone-number match.
2. Exact email/iMessage handle match.
3. Source-native contact identifier if available.
4. Fall back to the raw handle/phone number.

Normalize phone numbers before matching while preserving the original stored value. Avoid fuzzy identity matching in the initial implementation because wrong client attribution is worse than an unresolved number.

Example result:

```text
Before:
+3069XXXXXXXX: Do you still have Calacatta Light in 2 cm?

After contact resolution:
Nikos Papadopoulos — ABC Marble Ltd (+3069XXXXXXXX):
Do you still have Calacatta Light in 2 cm?
```

## Apple Backup Importer

### Discovery
The importer should accept a user-selected iPhone backup directory rather than hardcoding a Windows path.

Read `Manifest.db` and resolve backup file IDs by logical domain + relative path.

Minimum files to resolve:

```text
HomeDomain/Library/SMS/sms.db
HomeDomain/Library/AddressBook/AddressBook.sqlitedb
```

Optional:

```text
HomeDomain/Library/AddressBook/AddressBookImages.sqlitedb
MediaDomain/Library/SMS/Attachments/**
```

### Encrypted backups
Encrypted Apple backups should be treated as a separate implementation phase unless an existing audited dependency can be used safely.

Requirements:
- Never store the backup password in plaintext settings.
- Never silently fall back to partial ingestion when decryption fails.
- Clearly report "encrypted backup unsupported" or "password/decryption failed" until proper support exists.

### Message extraction
The Apple importer must preserve:
- thread/chat identifier
- timestamps
- sender/recipient handles
- incoming/outgoing direction
- text
- service when available (`iMessage`, `SMS`, `RCS`)
- attachments metadata when resolvable
- group-thread participant membership

Do not assume one database schema forever. Detect required tables/columns and fail with a useful compatibility message when Apple changes the schema.

## Viber Importer

### Discovery
Allow the user to select the Viber database or Viber data directory. Do not hardcode a user profile path.

### Extraction goals
Preserve:
- conversation/thread id
- participant/contact id
- display name when available
- phone number when available
- timestamps
- incoming/outgoing direction
- message text
- group-chat membership/title
- attachment/file metadata where recoverable

### Schema robustness
Viber's local schema is implementation-specific and may change. The reader should use schema inspection and version-tolerant queries rather than depending on one undocumented layout without validation.

If the DB is locked by Viber, prefer a safe read-only snapshot/copy strategy rather than writing to or modifying the live database.

## Archive Strategy
Business message threads should become searchable HalluScribe sessions.

Recommended archive identity:

```text
source + conversation_id + bounded time window
```

Do not automatically represent a multi-year client chat as one enormous session. Split long-running chats into deterministic windows, for example by inactivity gap or calendar period, while preserving the same conversation/contact identity.

Recommended first rule:
- split when inactivity exceeds 30 days
- otherwise cap by a maximum message/token threshold

This keeps raw reads useful and prevents giant transcripts from overwhelming summary generation.

## Raw Transcript Format
The preserved raw representation should remain human-readable and deterministic, for example:

```text
[2026-04-14 10:32] CLIENT | Nikos Papadopoulos | +3069XXXXXXXX
Do you still have Calacatta Light in 2 cm?

[2026-04-14 10:35] ME
Yes, I have three slabs available.
```

This format makes literal search for terms such as `Calacatta Light`, order codes, dimensions, prices, names, and phone numbers straightforward.

The original source database remains the authoritative source; HalluScribe's raw copy is an ingestion artifact.

## Search Behavior
The existing MCP hierarchy should be sufficient if imported message threads are indexed correctly.

Example request:

> "A client a few months ago ordered Calacatta Light. Find the conversation and tell me what happened."

Expected agent flow:

1. `search_raw_transcripts("Calacatta Light")`
2. Inspect candidate thread/session metadata.
3. `read_raw_session` on the strongest candidate(s).
4. If necessary, search/read adjacent sessions from the same conversation/contact.
5. Answer with the client identity, dates, order details, and outcome found in the messages.
6. Distinguish explicit facts from inference.

Do not require the model to guess the customer from a summary if an exact raw match exists.

## UI / Settings
Add a source configuration section for business messaging imports.

Suggested controls:
- Enable Viber ingestion
- Viber database/data path + Browse
- Enable Apple backup ingestion
- Apple backup directory + Browse
- Import Messages
- Import Contacts
- Import attachments metadata
- Last successful import timestamp
- Last error / compatibility warning
- Run import now

If multiple Apple backups exist, store a stable device/backup identity rather than assuming the newest directory is always correct.

## Privacy and Safety
These sources contain communications from third parties, so defaults should be conservative.

Requirements:
- Read-only source access.
- No modification of Viber databases or Apple backups.
- No cloud upload.
- Business-message sources should be opt-in.
- Exclude imported message data from persona-pack export by default.
- Preserve existing redaction behavior for MCP-facing archive reads.
- Clearly warn that raw transcripts may contain credentials, phone numbers, addresses, invoices, and other sensitive business/customer data.

## Profile Behavior
Do not automatically let customer-message history dominate the existing Personal profile.

Recommended approach:
- Add source/category metadata such as `business_message`.
- Initially keep business messages searchable through sessions/raw transcripts.
- Add profile integration only after validating that it does not pollute identity/style summaries.
- If profile integration is added, prefer a dedicated `business` scope rather than silently mixing all client conversations into `personal`.

A future business profile could contain:
- recurring customers
- suppliers
- products/materials discussed
- recurring operational issues
- business conventions
- active projects/orders

But exact commercial facts should still be retrieved from sessions/raw data rather than treated as profile memory.

## Deduplication and Incremental Import
Importers must be incremental.

Use stable source identifiers where available:
- source message id
- source conversation id
- device/backup identity
- timestamp + sender + content hash only as a last-resort fallback

Do not re-summarize unchanged historical message segments on every sweep.

Track a per-source ingestion cursor/state so a new Apple backup or updated Viber DB only adds or updates changed material.

## Attachments
Phase 1 should index attachment metadata, not attempt broad content understanding.

Store where available:
- filename
- MIME/type
- source path/reference
- message association
- timestamp

Later phases can add OCR/document parsing/image understanding as separate capabilities.

## Implementation Phases

### Phase A — Investigation and fixtures
- Inspect current HalluScribe reader/archive interfaces before adding code.
- Obtain anonymized/synthetic fixture DBs for Viber and Apple Messages/Contacts.
- Document actual schemas encountered.
- Confirm how imported private-chat sources currently become archive sessions so this feature reuses the same pipeline.

### Phase B — Apple unencrypted backup MVP
- Backup directory selection.
- `Manifest.db` resolver.
- `sms.db` extraction.
- `AddressBook.sqlitedb` extraction.
- Exact handle-to-contact resolution.
- Normalize to message sessions.
- Preserve raw transcript.
- Feed existing summary/archive/index pipeline.
- Tests using fixtures.

### Phase C — Viber Desktop MVP
- DB/directory selection.
- Schema detection.
- Read-only/snapshot access.
- Conversation/contact/message extraction.
- Normalize to the same model.
- Raw transcript + archive/index integration.
- Tests using fixtures.

### Phase D — Incremental ingestion and UX
- Per-source checkpoints.
- Import status/errors in Settings.
- Manual "Run import now" action.
- Deduplication.
- Long-thread segmentation.

### Phase E — Encrypted Apple backups
- Evaluate a maintained, auditable implementation/dependency.
- Password handling design.
- Decrypt only in memory or controlled temporary storage.
- Add explicit tests for wrong-password and unsupported-version cases.

### Phase F — Attachments and business profile
- Attachment metadata/indexing.
- Optional dedicated `business` profile/digest scope.
- Optional entity layer for customers/products/orders only if it can remain traceable to source messages.

## Tests / Acceptance Criteria

### Apple
- Locate `sms.db` through `Manifest.db` in an unencrypted backup fixture.
- Read one-to-one and group conversations.
- Resolve at least one phone handle to a contact.
- Preserve unresolved handles accurately.
- Distinguish incoming vs outgoing messages.
- Preserve timestamps accurately.
- Re-import without duplicate sessions/messages.

### Viber
- Read a fixture without modifying it.
- Extract personal and group conversations.
- Preserve names/numbers where present.
- Preserve message order and direction.
- Fail clearly on an unsupported schema.
- Re-import without duplication.

### HalluScribe / MCP
Given a fixture containing the phrase `Calacatta Light`:

1. `search_raw_transcripts("Calacatta Light")` returns the correct imported business conversation.
2. `read_raw_session` returns the surrounding conversation with timestamps and resolved client identity when available.
3. The archived summary does not replace or destroy the raw exact wording.
4. Existing coding-session ingestion continues to behave unchanged.

## Non-goals for the MVP
- Sending or replying to messages.
- Live message monitoring.
- Writing to source databases.
- CRM automation.
- Automatic invoice/order creation.
- Fuzzy identity merging across unrelated contacts.
- Cloud synchronization.

The first release should remain a local, read-only historical-memory extension to HalluScribe.

## Research Notes / External Formats
Current implementation research indicates these are the useful source artifacts to validate against real fixtures before coding:

- Apple local backup manifest: `Manifest.db`
- Apple Messages logical database: `HomeDomain/Library/SMS/sms.db`
- Apple Contacts logical database: `HomeDomain/Library/AddressBook/AddressBook.sqlitedb`
- Apple contact images: `HomeDomain/Library/AddressBook/AddressBookImages.sqlitedb`
- Apple message attachments: under the backup's `Library/SMS/Attachments` mapping
- macOS alternative: `~/Library/Messages/chat.db`
- Viber Desktop: local SQLite database commonly exposed as `viber.db` in existing tooling

These paths/formats are implementation details, not contracts. The implementation must validate them against current real data and fail explicitly when schemas differ.

## Recommended First Implementation
Start with **Apple unencrypted local backup + Contacts** before encrypted backup support. It has a clean read-only boundary and directly supports the business-memory use case.

Implement Viber next using the same normalized message-thread interface. Once both adapters emit identical normalized sessions, the existing HalluScribe archive, raw search, and MCP layers should require minimal special casing.
