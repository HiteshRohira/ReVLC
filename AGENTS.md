When the user asks for an audit or QA pass, follow docs/audit-playbook.md.

Whenever you launch or test the app, keep analytics off so agent runs don't reach PostHog: set `"analytics": false` in settings.json before the first launch (`python3 -I scripts/qa/appdata.py seed <run dir>` does this after a backup), and never set `REVLC_ANALYTICS_SEND`.
