---
name: gcal-browser
description: Configure Google Calendar Browser with /connect and use Calendar ConnectorAct actions through the global Puffer browser profile.
allowed-tools:
  - Bash
argument-hint: "[Google Calendar Browser task]"
arguments: target
user-invocable: true
disable-model-invocation: false
---

Use `/connect gcal-browser <connection>` when the user needs Google Calendar
Browser setup or auth repair. The flow opens Google Calendar in the global
Puffer browser profile, lets the user sign in, discovers logged-in Google
accounts, and saves the accounts selected for monitoring.

Target: $target

Configuration command:

```text
/connect gcal-browser <connection>
```

Use `ConnectorAct` for configured Google Calendar Browser connections instead
of opening a separate browser session. Google Calendar Browser actions are:

- `get_detail` to open a monitored event and return the visible event details.
- `accept` to RSVP yes to a Calendar invitation.
- `deny` to RSVP no or decline a Calendar invitation.
- `requestuserbrowseraction` only when the connector needs the user to complete
  sign-in or approval in the global Puffer browser profile.

Newly observed visible Calendar events are emitted by the subscriber every 30
seconds. Include `account` with the Google account address when a connection
monitors multiple accounts. Event actions need one of `event_id`, `title`, or
`url`; event payloads emitted by the subscriber include `event.id` and `url`.

Connector action examples:

```json
{"connector_slug":"gcal-browser","connection_slug":"gcal-browser","action":"get_detail","input":{"account":"cs@agentenv.io","event_id":"event-123"}}
{"connector_slug":"gcal-browser","connection_slug":"gcal-browser","action":"accept","input":{"account":"cs@agentenv.io","event_id":"event-123"}}
{"connector_slug":"gcal-browser","connection_slug":"gcal-browser","action":"deny","input":{"account":"cs@agentenv.io","url":"https://calendar.google.com/calendar/event?..."}}
```

Do not use Gmail action names such as `list_emails` or `draft_reply` for
Google Calendar Browser. Use only the Calendar action surface above.
