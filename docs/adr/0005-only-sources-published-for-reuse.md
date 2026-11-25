---
status: accepted
---

# Use only Sources that publish their data for reuse

"Publicly available" covers published open data, sites that are public but forbid automated collection, and collections that were scraped or leaked. We decided that Orion uses only Sources that publish their data for reuse under stated terms, stays within those terms, and credits them. Data that is reachable but not offered for reuse is not used, whether that means scraping against a site's terms or taking a leaked collection.

## Consequences

- Some Layers people will ask for cannot exist. When this was decided, live road congestion had no open Source, and the best-known lightning and flight networks forbade this kind of use.
- Orion stays non-commercial, which keeps Sources that allow only non-commercial use available.
- Adding a Layer starts with reading its Source's terms, and each Layer names its Source in the app.
- The best Source is sometimes out of reach. Unrest Reports come from GDELT, which is built from news by machine and is often wrong, because the curated record of protests, ACLED, forbids giving users access to its data, and nobody publishes planned demonstrations or strikes for reuse. Germany's motorway incidents are left out until their publisher states a licence.
- Road Congestion has no Layer. No open Source covers the world, and the commercial one looked at in November 2026, TomTom, offers a free allowance to anyone who registers a key but licenses its data only for applications that manage mobile assets by tracking or routing them, or for testing its products. Orion does neither and will not (ADR 0002), so a user's own key would not make the use a licensed one. Its terms also forbid keeping results beyond what each response allows. The same key would have brought accidents worldwide; those stay limited to what road authorities publish.
