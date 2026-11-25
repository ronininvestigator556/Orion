# Orion

A desktop app that shows every publicly catalogued object orbiting Earth on a 3D globe, at its computed position for any chosen moment.

## Language

### What is in orbit

**Tracked Object**:
Anything in orbit that appears in the Catalogue: a Satellite, a Rocket Body or Debris.
_Avoid_: Satellite (as the general word), space object, sat

**Satellite**:
A Tracked Object that was launched as a payload, whether or not it still works.
_Avoid_: Payload, spacecraft, bird

**Rocket Body**:
A Tracked Object that is a spent launch-vehicle stage.
_Avoid_: Booster, upper stage, R/B

**Debris**:
A Tracked Object that is a fragment or discarded part, never a whole Satellite or Rocket Body.
_Avoid_: Junk, fragment, space trash

**Catalogue**:
The public list of Tracked Objects, each with a permanent catalogue number. Some entries have no published Element Set, because the object is restricted or has been lost.
_Avoid_: Database, satellite list, SATCAT

**Group**:
A named set of Tracked Objects that share a purpose or belong to one constellation, such as weather Satellites or Starlink.
_Avoid_: Category, collection, constellation (as the general word)

### Where things are

**Element Set**:
The published description of one Tracked Object's orbit, valid around its Epoch.
_Avoid_: TLE, orbital data, ephemeris

**Epoch**:
The moment an Element Set describes.
_Avoid_: Timestamp, element date

**Element Age**:
The time between an Element Set's Epoch and the Simulation Clock; the larger it is, the less a Position can be trusted.
_Avoid_: Staleness, data age

**Position**:
Where a Tracked Object is computed to be at a given moment; an estimate derived from an Element Set, never a measurement.
_Avoid_: Location, live position, real position

**Propagation**:
Computing a Position from an Element Set for a given moment.
_Avoid_: Tracking, simulation, prediction

**Simulation Clock**:
The moment the app is displaying, which follows real time by default and can be paused, sped up or moved.
_Avoid_: Current time, app time, now

**Orbit Path**:
The closed curve a Tracked Object traces through space over one revolution.
_Avoid_: Orbit line, trajectory, trail

**Ground Track**:
The line on Earth's surface directly beneath a Tracked Object as it moves.
_Avoid_: Footprint, ground path, trace

**Orbit Regime**:
The broad class of a Tracked Object's orbit by altitude and shape: low, medium, geostationary or highly elliptical.
_Avoid_: Orbit type, orbit class, altitude band

### Looking at the globe

**Earth-Fixed View**:
A view in which Earth stays still and Tracked Objects move over it.
_Avoid_: Geographic view, rotating frame

**Star-Fixed View**:
A view in which the stars stay still and Earth rotates beneath the orbits.
_Avoid_: Inertial view, space view

**Follow Mode**:
A view that keeps the selected Tracked Object centred as it moves.
_Avoid_: Tracking mode, lock-on, chase camera

### Watching from the ground (planned, not in the first version)

**Observer**:
A fixed place on Earth's surface from which Tracked Objects are watched.
_Avoid_: User location, ground station, site

**Pass**:
One continuous period during which a Tracked Object is above an Observer's horizon.
_Avoid_: Flyover, overpass, visibility window
