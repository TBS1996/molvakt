# molvakt WhatsApp Flow — menu

Multi-screen menu Flow that replaces the interactive list menu. Dynamic data (active chat summary, conversation list) comes from your **data_exchange endpoint**.

## Setup in Meta

1. WhatsApp Manager → **Flows** → Create flow (or paste `menu.json` in the JSON editor).
2. Set **Endpoint URL** to `https://<your-host>/webhook/whatsapp-flow`
3. Upload the **RSA public key** Meta gives you; keep the matching private key in `WHATSAPP_FLOW_PRIVATE_KEY` (PEM, PKCS#8).
4. Publish the flow and note the **Flow ID**.
5. Set `WHATSAPP_MENU_FLOW_ID` in your env.

## Sending the flow

When the user sends `MENU`, send an interactive flow message (24h window):

```json
{
  "type": "interactive",
  "interactive": {
    "type": "flow",
    "body": { "text": "Open the molvakt menu" },
    "action": {
      "name": "flow",
      "parameters": {
        "flow_message_version": "3",
        "flow_id": "<WHATSAPP_MENU_FLOW_ID>",
        "flow_token": "<signed-or-random-session-token>",
        "flow_cta": "Open menu",
        "flow_action": "data_exchange",
        "mode": "published"
      }
    }
  }
}
```

`flow_token` must identify the user — the endpoint maps it to a phone number (e.g. store `flow_token → phone` when sending, or embed phone if you sign it).

Use `flow_action: "data_exchange"` so opening the flow triggers an **INIT** request to your endpoint and loads the MAIN screen with live data.

## Endpoint contract

All requests are encrypted by Meta (see [Implementing your flow endpoint](https://developers.facebook.com/docs/whatsapp/flows/guides/implementingyourflowendpoint)). After decryption:

```json
{
  "version": "3.0",
  "action": "INIT | data_exchange | BACK",
  "screen": "MAIN | PICK_CHAT | ...",
  "flow_token": "<token>",
  "data": { }
}
```

### INIT → MAIN

Load user's active chat summary and action list.

**Response:**

```json
{
  "screen": "MAIN",
  "data": {
    "summary": "Current chat: Mel\nExchange (turns) — you learn French…",
    "actions": [
      { "id": "menu_list", "title": "View conversations", "description": "See all your chats" },
      { "id": "menu_switch", "title": "Switch chat", "description": "Change active conversation" }
    ]
  }
}
```

Build `summary` with the same logic as `format_menu_body()` in `conversations.rs`.  
Omit actions the user can't use (e.g. no `menu_cancel` if they have no pending invites).

### data_exchange — `trigger: main_action`

Payload: `{ "trigger": "main_action", "action": "menu_switch" }`

| `action` | Response |
|---|---|
| `menu_switch` | `{ "screen": "PICK_CHAT", "data": { "menu_action": "switch", "prompt": "…", "conversations": [...] } }` |
| `menu_cancel` | Same with `menu_action: "cancel"` and filtered listings |
| `menu_set_mode` | `{ "screen": "PICK_MODE", "data": { "partner": "Mel" } }` or `{ "screen": "MAIN", "data": { ..., "error_message": "No active chat…" } }` |
| `menu_set_language` | `{ "screen": "SET_LANGUAGE", "data": { "partner": "Mel" } }` |
| `menu_start` | `{ "screen": "START_NEW", "data": {} }` |
| `menu_list` | Close flow — see below |
| `menu_help` | Close flow |
| `menu_review_vocab` | Close flow |

**Conversation row shape** (max 200 items, title ≤ 30 chars):

```json
{ "id": "<conversation_id>", "title": "1. Mel*", "description": "Exchange (turns), waiting…" }
```

Use `listing_menu_labels_owned()` / `format_listing_menu_description()` from `menu.rs`.

### data_exchange — `trigger: chat_picked`

Payload: `{ "trigger": "chat_picked", "menu_action": "switch", "conversation_id": "42" }`

**Close the flow** (no extra confirmation screen):

```json
{
  "screen": "SUCCESS",
  "data": {
    "extension_message_response": {
      "params": {
        "flow_token": "<same token>",
        "action": "switch",
        "conversation_id": "42"
      }
    }
  }
}
```

Same pattern for `menu_list`, `menu_help`, `menu_review_vocab` with `action` set accordingly.

### Terminal screens (no endpoint)

These screens use `complete` and send `nfm_reply` to your normal WhatsApp webhook:

| Screen | Payload |
|---|---|
| `PICK_MODE` | `{ "action": "set_mode", "mode": "menu_mode_learner" }` |
| `SET_LANGUAGE` | `{ "action": "set_language", "language": "Norwegian" }` |
| `START_NEW` | `{ "action": "start_new", "start_mode": "mode_learner" }` |

Handle `interactive.type === "nfm_reply"` in `whatsapp.rs` and route to the same handlers as today's list menu IDs.

## Screen map

```
MAIN (INIT loads summary + actions)
 ├─ switch/cancel ──► PICK_CHAT ──► SUCCESS (close)
 ├─ set_mode ───────► PICK_MODE ──► complete
 ├─ set_language ───► SET_LANGUAGE ► complete
 ├─ start_new ──────► START_NEW ───► complete
 └─ list/help/review ► SUCCESS (close)
```

## IDs match existing menu

Action IDs intentionally match `src/menu.rs` constants (`menu_switch`, `mode_learner`, etc.) so the bot can reuse `handle_menu_selection()` after parsing the flow response.
