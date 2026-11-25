---
status: accepted
---

# A scheduled job in the repository publishes Extracts

Orion has no server: each copy fetches from Sources directly. That does not work for places taken from OpenStreetMap, such as licence plate cameras, data centres and mines. OpenStreetMap's query service asks apps not to use it as their backend, and the raw data is far too large for each copy to filter itself. We decided that a scheduled job in the GitHub repository collects those places and publishes them as small Extract files attached to a release, which every copy downloads.

## Considered Options

- **Every copy queries OpenStreetMap's public query service**: no upkeep, but it is exactly the use that service asks apps to avoid.
- **A server of our own**: flexible, but a running cost and an operational duty that a hobby project should not take on.

## Consequences

- "No backend" now means no server. There is one pipeline to keep working, and if it stops, Extracts go stale but the app still runs.
- Extracts derived from OpenStreetMap stay under its Open Database Licence, separate from the code's licence, and the app must show "© OpenStreetMap contributors" whenever such a Layer is on.
- Sources that need a personal key, such as live ship positions, are used only with a key each user supplies. The project does not hold keys on anyone's behalf.
- The same arrangement serves a Source whose raw files are too large or too rough for each copy to sift, where its terms allow republishing. Unrest Reports are sifted this way every hour from GDELT's index of the news, which would otherwise cost each copy several megabytes a day.
