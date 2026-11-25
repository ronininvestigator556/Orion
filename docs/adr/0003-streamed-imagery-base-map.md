---
status: accepted
---

# Stream the Base Map as imagery tiles instead of bundling one Earth image

The first version drew Earth from a single bundled image, which is enough to watch satellites from orbit but dissolves into a blur long before a town is visible. Layers of places on the ground need the view to come much closer, to roughly 10 to 20 km up. We decided to stream the Base Map as satellite imagery tiles from NASA's GIBS service, which needs no key, and to draw borders, coastlines and major roads over it as lines from the public-domain Natural Earth set, which is small enough to bundle.

## Considered Options

- **Keep the bundled image**: simplest, and works offline, but rules out every Layer that only makes sense close up.
- **A full street map from OpenStreetMap vector tiles**: every road and label, but it is a mapping engine in its own right, and the only keyless host is donation-funded with no guarantees. The whole map is 90 to 140 GB, so bundling it is not possible.

## Consequences

- The globe needs a connection to look sharp. The bundled image stays as what is shown before tiles arrive and when offline.
- There are no street names or minor roads. If Layers such as cameras turn out to need street context, the street map option is revisited then.
- The camera must be able to come far closer than its first limit of about 640 km.
