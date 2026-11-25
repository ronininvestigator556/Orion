---
status: accepted
---

# Lay OpenStreetMap's ready-made tiles over the globe for the street map

ADR 0003 left the Base Map without streets, to be revisited when a Layer needed them. Unrest Reports and Road Incidents do: knowing that a road is closed or a march was reported is little use without seeing which streets are meant. We decided that, close up, the Base Map becomes a street map made of the ready-drawn tiles that OpenStreetMap's own server publishes without a key, laid over the globe. Far out it stays imagery.

## Considered Options

- **Draw the map ourselves from vector tiles**: four hosts now publish the roads and names as data without a key. The map would be in Orion's own dark style, sharp at any size, with names in Latin script everywhere. But drawing every road, and above all placing names along curved streets on a sphere, is a mapping engine in its own right, and no library in Rust does that placing. It is the largest piece of work Orion would have taken on.
- **Each country's own mapping agency**: France and the United States publish keyless street tiles with firmer promises than a donation-funded host makes. The map would look different in each country and stop at each border.
- **A commercial host's free allowance**: dependable, but every user would need to register a key before seeing a street.

## Consequences

- Street names appear as they are written where they are: in Arabic, Chinese or Cyrillic script in those countries. The map's look is OpenStreetMap's, light-coloured, and cannot be restyled to match the rest of Orion.
- The host is funded by donations and promises nothing. The address tiles are fetched from is therefore a setting, not fixed in the code. Orion names itself in each request, keeps every tile on disk for at least a week, and never fetches tiles ahead of the view or in bulk, as the host's policy asks.
- "© OpenStreetMap contributors" is shown whenever the street map is.
- These tiles are cut on a different grid from the imagery and stop short of the poles, so they are laid onto the globe by a second path, and imagery remains what is shown near the poles.
- The camera must come down to a kilometre or two, far closer than the 25 km it stopped at.
- This replaces ADR 0003's consequence that there are no street names or minor roads. The user can still choose imagery close up.
- Drawing the map ourselves stays open as a later step, if the local scripts or the look prove to matter.
