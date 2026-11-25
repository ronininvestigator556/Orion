#!/usr/bin/env python3
"""Gathers the work zones under way on U.S. roads into the one file that Orion reads.

Each U.S. state's transport department decides for itself whether to publish its work
zones, and the U.S. Department of Transportation keeps a list of those that do, in the
shared Work Zone Data Exchange format:
https://data.transportation.gov/d/69qe-yiui

There are dozens of lists, some several megabytes, so every copy of Orion fetching
each would be slow and unkind to the states. This job fetches them once an hour and
publishes what is under way now (ADR 0004).

Run with Python 3 and nothing else installed:

    python3 tools/make_road_works.py <folder>

It writes <folder>/road_works_us.tsv. Only lists that need no key are asked, and only
those that state the open licence the format names are used; the rest are reported
and left out. A list that cannot be fetched is reported and the others still used.

These are work zones. No state publishes its accidents this way.
"""

import dataclasses
import datetime
import json
import os
import sys
import urllib.request

REGISTRY = "https://data.transportation.gov/resource/69qe-yiui.json?$limit=500"
# Says who is asking, without saying more than that.
USER_AGENT = "orion-road-works/1"
HEADING = "#orion-road-works"
VERSION = "1"
# The licence the Work Zone Data Exchange format has publishers state.
OPEN_LICENCE = "creativecommons.org/publicdomain/zero"
# The most of a state's description that is kept.
DESCRIPTION_LETTERS = 300
# A list larger than this is not a list of work zones.
MAX_BYTES = 60 * 1024 * 1024


class NotOpen(Exception):
    """A list that does not state the open licence."""


@dataclasses.dataclass(frozen=True)
class WorkZone:
    id: str
    lat: float
    lon: float
    kind: str
    state: str
    road: str
    direction: str
    description: str
    start: str
    end: str
    publisher: str


def tidy(text, limit=None):
    """Text on one line, without the tabs that would break the file it goes into."""
    text = " ".join(str(text or "").split())
    if limit and len(text) > limit:
        text = text[: limit - 1].rstrip() + "…"
    return text


def moment(text):
    """A moment as the lists write it, or None if it cannot be read."""
    try:
        read = datetime.datetime.fromisoformat(str(text).replace("Z", "+00:00"))
    except ValueError:
        return None
    if read.tzinfo is None:
        read = read.replace(tzinfo=datetime.timezone.utc)
    return read


def written(read):
    return read.astimezone(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ") if read else ""


def place(geometry):
    """Where to mark a work zone: its point, or the middle of its line."""
    kind = (geometry or {}).get("type")
    points = (geometry or {}).get("coordinates") or []
    if kind == "Point":
        points = [points]
    elif kind == "MultiPoint":
        points = points[:1]
    elif kind != "LineString":
        return None
    try:
        lon, lat = points[len(points) // 2][:2]
        return float(lat), float(lon)
    except (IndexError, TypeError, ValueError):
        return None


def keyless_feeds(registry_text):
    """The lists that can be asked: state, short name, publisher and address."""
    feeds = []
    for entry in json.loads(registry_text):
        truthy = lambda value: str(value).lower() == "true"
        address = (entry.get("url") or {}).get("url", "")
        if not truthy(entry.get("active")) or truthy(entry.get("needapikey")):
            continue
        if entry.get("format") != "geojson" or not address.startswith("https://"):
            continue
        feeds.append(
            (
                entry.get("state", ""),
                entry.get("feedname", ""),
                entry.get("issuingorganization", ""),
                address,
            )
        )
    return feeds


def work_zones(text, state, feed_name, publisher, now):
    """The work zones under way at `now` in one state's list."""
    document = json.loads(text)
    info = document.get("feed_info") or document.get("road_event_feed_info") or {}
    licences = [str(info.get("license", ""))]
    licences += [str(source.get("license", "")) for source in info.get("data_sources") or []]
    if not any(OPEN_LICENCE in licence for licence in licences):
        raise NotOpen(feed_name)

    found = []
    for feature in document.get("features") or []:
        facts = feature.get("properties") or {}
        # Since version 4 the facts every kind of event has sit in a group of their own.
        core = facts.get("core_details") or facts
        if core.get("event_type") != "work-zone":
            continue
        where = place(feature.get("geometry"))
        start = moment(facts.get("start_date"))
        end = moment(facts.get("end_date")) if facts.get("end_date") else None
        if not where or not start or start > now or (end and end < now):
            continue
        roads = core.get("road_names") or [core.get("road_name") or ""]
        event_id = feature.get("id") or facts.get("road_event_id") or core.get("name")
        if not event_id:
            continue
        found.append(
            WorkZone(
                id=tidy(f"{feed_name}/{event_id}"),
                lat=where[0],
                lon=where[1],
                kind="closure" if facts.get("vehicle_impact") == "all-lanes-closed" else "roadworks",
                state=tidy(state).title(),
                road=tidy(", ".join(road for road in roads if road)),
                direction=tidy(core.get("direction")),
                description=tidy(core.get("description"), DESCRIPTION_LETTERS),
                start=written(start),
                end=written(end),
                publisher=tidy(publisher),
            )
        )
    return found


def write(path, built, found):
    with open(path, "w", encoding="utf-8") as file:
        file.write(f"{HEADING}\t{VERSION}\t{built}\t{len(found)}\n")
        for one in found:
            fields = dataclasses.astuple(one)
            file.write(
                "\t".join(f"{field:.5f}" if isinstance(field, float) else field for field in fields)
            )
            file.write("\n")


def get(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=90) as response:
        return response.read(MAX_BYTES).decode("utf-8", "replace")


def main(arguments):
    if len(arguments) != 1:
        sys.exit(__doc__)
    folder = arguments[0]
    os.makedirs(folder, exist_ok=True)
    now = datetime.datetime.now(datetime.timezone.utc)

    feeds = keyless_feeds(get(REGISTRY))
    found, used = [], 0
    for state, feed_name, publisher, address in feeds:
        try:
            here = work_zones(get(address), state, feed_name, publisher, now)
        except NotOpen:
            print(f"  left out, states no open licence: {feed_name} ({state})")
            continue
        except Exception as error:  # A state's list being down must not stop the rest.
            print(f"  could not be read: {feed_name} ({state}): {type(error).__name__}")
            continue
        used += 1
        found.extend(here)
        print(f"  {feed_name} ({state}): {len(here)}")
    if not used:
        sys.exit(f"none of the {len(feeds)} lists could be used")

    built = now.strftime("%Y-%m-%dT%H:%M:%SZ")
    write(os.path.join(folder, "road_works_us.tsv"), built, found)
    closed = sum(1 for one in found if one.kind == "closure")
    print(f"{len(found)} work zones under way from {used} of {len(feeds)} lists; {closed} close the road")


if __name__ == "__main__":
    main(sys.argv[1:])
