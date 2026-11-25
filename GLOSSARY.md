# Orion

A desktop app that shows publicly available information about the world on a 3D globe, in Layers that can be switched on and off: what is in orbit, and places, traffic, weather and events on and above the surface. It shows places, vehicles and events as their Sources publish them, and is not a tool for following individuals.

## Language

### What is on the globe

**Layer**:
One kind of thing from one kind of source, shown or hidden as a whole, with its own colour, legend and filter.
_Avoid_: Overlay, dataset, category

**Theme**:
A heading under which related Layers are listed: Orbit, Places, Energy, Roads, Traffic, Weather, Hazards, Unrest, Conflict or Imported. Traffic is the Theme of vehicles shown moving, such as aircraft and ships; what is happening on the roads themselves belongs to Roads. Weather is the Theme of pictures of the sky and sea; an earthquake, a storm or an official warning belongs to Hazards.
_Avoid_: Group, category, folder, section

**Feature**:
One thing shown on a Layer: a fixed place such as a mine, a route such as a pipeline, an area such as an oil field, something moving such as a ship, or an event such as an earthquake or an Unrest Report. A Tracked Object is a Feature of the orbit Layer.
_Avoid_: Entity, item, marker, point of interest

**Base Map**:
The picture of Earth's surface drawn beneath every Layer, which sharpens as the view moves closer, together with the borders, major roads and place names that make it readable. Far out it is imagery; close up it gives way to a street map.
_Avoid_: Background, tiles, imagery layer

**Cluster**:
A single mark standing for many nearby Features of one Layer, showing how many, used when the view is too far out to tell them apart.
_Avoid_: Group, bubble, aggregate, heat map

**Source**:
Whoever publishes the data behind a Layer, under terms that allow it to be reused.
_Avoid_: Provider, feed, API

**Listing**:
A file of Features for one Layer that a Source itself publishes and keeps replacing as things change, fetched by the app as it is.
_Avoid_: Feed, endpoint, API

**Fire Detection**:
A hot spot on the ground seen by a satellite's instrument, about a kilometre across. It may be a wildfire, a planned burn, a gas flare or a volcano, so it is never called a fire outright.
_Avoid_: Wildfire, fire, hotspot

**Unrest Report**:
A protest, a protest with violence or a strike as news articles reported it, in a town and on a day. It is a report, not an observation: it may be late, placed no more exactly than the town, or wrong, and it never says where a crowd is at this moment or what is planned.
_Avoid_: Protest or riot (as the general word), civil emergency, incident, demonstration

**Armed Conflict**:
Fighting between named parties, states or organised armed groups, that a Source follows under one name from month to month.
_Avoid_: War (as the general word), crisis, hostilities

**Conflict Event**:
One incident of organised violence in which at least one person died, as a Source recorded it: where, on what day, between whom, and how many are estimated to have died. It is one of three kinds: fighting that involves a state, fighting between armed groups, or an attack on civilians.
_Avoid_: Battle, attack (as the general word), terrorism, incident

**Navigation Warning**:
A government's notice to ships that an area of sea is dangerous for a time, as for missile firing, gunnery or mines.
_Avoid_: Exclusion zone, closed area, NAVAREA, alert

**Airspace Warning**:
An aviation authority's notice that a country's airspace is dangerous to fly through because of a conflict.
_Avoid_: No-fly zone, NOTAM, alert

**Travel Advisory**:
A government's published advice about travelling to a country, with the level of caution it sets. It speaks for the whole country, or names parts of it in words.
_Avoid_: Travel warning, alert, risk level

**Road Incident**:
Something getting in the way on a road, as the road's authority published it: an accident, a closure or roadworks.
_Avoid_: Traffic, traffic event, hazard, alert

**Road Congestion**:
How slowly vehicles are moving along a stretch of road compared with when it is clear.
_Avoid_: Traffic, traffic flow, jam

**Extract**:
A file of Features for one Layer, prepared ahead of time by the project from a Source and downloaded by the app.
_Avoid_: Dump, export, dataset

**Imported Layer**:
A Layer made from a file of places that the user opens, as opposed to one that comes with the app.
_Avoid_: Custom layer, user layer, overlay

**Time Coverage**:
The moments a Layer is able to show: any moment, only the present, or the present and some of the past.
_Avoid_: Time range, availability, history

### What is in orbit

**Tracked Object**:
Anything in orbit that appears in the Catalogue: a Satellite, a Rocket Body or Debris. The word is reserved for orbit; an aircraft or ship is a Feature, not a Tracked Object.
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

**Catalogue Record**:
The Catalogue's lasting facts about one Tracked Object: its owner, launch date and launch site.
_Avoid_: Metadata, SATCAT entry, catalogue entry

**Group**:
A named set of Tracked Objects that share a purpose or belong to one constellation, such as weather Satellites or Starlink. Not a set of Layers; that is a Theme.
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

**Snapshot**:
A set of Element Sets captured at one moment and shipped inside the app, used when no fresher ones are available.
_Avoid_: Bundle, seed data, offline data

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
