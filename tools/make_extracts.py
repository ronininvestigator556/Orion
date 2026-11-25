"""Builds Orion's Extracts: small files of places for one Layer each, taken from
OpenStreetMap.

OpenStreetMap's query service must not be used as the backend of an app, so Orion
never queries it. This script does, a handful of times, from a scheduled job in the
repository (see .github/workflows/extracts.yml), and the job publishes what it writes.
Every copy of Orion downloads those files instead.

The data is (c) OpenStreetMap contributors and stays under the Open Database Licence.

Run with Python 3 and nothing else installed:

    python3 tools/make_extracts.py <folder to write to> [layer ...]

With no layer named, every Extract is built. Set ORION_EXTRACTS_CONTACT to an address
or page where the query service's operators can reach whoever runs the job.

Each Extract is a text file of tab-separated lines. The first names the format, its
version, the layer, the day it was built and how many places follow. Every other
line is one place: its OpenStreetMap id, latitude, longitude, class, name, and details
as `Label=value` pairs separated by `|`.
"""

import datetime
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

# The public query service. ORION_OVERPASS_ENDPOINT names another to use instead.
ENDPOINT = os.environ.get("ORION_OVERPASS_ENDPOINT", "https://overpass-api.de/api/interpreter")
# The service is often busy. A query it turns away is tried again after each of these
# waits, in seconds, before the run gives up.
RETRY_WAITS = (60, 120, 240, 480)
# Seconds the query service may spend on one query, and that this script waits for it.
QUERY_SECONDS = 900
# A pause between queries, out of courtesy to a free public service.
PAUSE_SECONDS = 20


def first(tags, *keys):
    """The value of the first of `keys` that the place is tagged with."""
    for key in keys:
        value = tags.get(key, "").strip()
        if value:
            return value
    return ""


def camera(tags):
    maker = first(tags, "manufacturer", "brand", "camera:manufacturer")
    operator = first(tags, "operator")
    known = " ".join((maker, operator, first(tags, "name"))).lower()
    if "flock" in known:
        kind, maker = 0, maker or "Flock Safety"
    elif maker:
        kind = 1
    else:
        kind = 2
    name = "Licence plate camera"
    if operator:
        name += f", {operator}"
    return kind, name, [
        ("Operator", operator),
        ("Maker", maker),
        ("Model", first(tags, "model", "camera:model")),
        ("Facing", first(tags, "direction", "camera:direction")),
        ("Mounted on", first(tags, "camera:mount", "support")),
        ("Watches", first(tags, "surveillance:zone", "surveillance")),
    ]


def data_centre(tags):
    operator = first(tags, "operator", "owner")
    name = first(tags, "name", "name:en") or (f"Data centre, {operator}" if operator else "Data centre")
    return 0, name, [
        ("Operator", operator),
        ("Place", first(tags, "addr:city", "addr:place")),
        ("Opened", first(tags, "start_date")),
        ("Website", first(tags, "website", "contact:website")),
    ]


def platform(tags):
    operator = first(tags, "operator", "owner")
    name = first(tags, "name", "name:en", "ref") or "Offshore platform"
    return 0, name, [
        ("Operator", operator),
        ("Produces", first(tags, "product", "substance", "resource")),
        ("Kind", first(tags, "platform:type", "offshore_platform", "description")),
        ("Opened", first(tags, "start_date")),
    ]


MILITARY_KINDS = {"base": (0, "Military base"), "airfield": (1, "Military airfield"), "naval_base": (2, "Naval base")}


def military(tags):
    kind, label = MILITARY_KINDS.get(tags.get("military", ""), (0, "Military base"))
    name = first(tags, "name", "name:en") or label
    return kind, name, [
        ("Kind", label),
        ("Operator", first(tags, "operator")),
        ("Branch", first(tags, "military_service", "military:branch")),
    ]


# Each Extract: the file's name, the query that finds its places, and the function
# that turns a place's tags into a class, a name and details.
EXTRACTS = {
    "cameras": (
        'nwr["man_made"="surveillance"]["surveillance:type"="ALPR"];',
        camera,
    ),
    "data_centres": (
        'nwr["telecom"="data_center"];nwr["building"="data_center"];',
        data_centre,
    ),
    "offshore_platforms": (
        'nwr["man_made"="offshore_platform"];',
        platform,
    ),
    "military_sites": (
        'nwr["military"~"^(base|airfield|naval_base)$"];',
        military,
    ),
}


def clean(text):
    """Text safe to put in one field: no tabs, line ends or the separators used inside it."""
    return " ".join(str(text).replace("|", "/").replace("=", ":").split())[:200]


def query(selection):
    """Asks the query service for every place a selection matches, with a point for each."""
    text = f"[out:json][timeout:{QUERY_SECONDS}];({selection});out center tags qt;"
    contact = os.environ.get("ORION_EXTRACTS_CONTACT", "").strip()
    agent = "orion-extracts/1" + (f" ({contact})" if contact else "")
    request = urllib.request.Request(
        ENDPOINT,
        data=urllib.parse.urlencode({"data": text}).encode(),
        headers={"User-Agent": agent},
    )
    for wait in (*RETRY_WAITS, None):
        try:
            with urllib.request.urlopen(request, timeout=QUERY_SECONDS + 60) as response:
                document = json.load(response)
            break
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
            # A refusal that waiting will not cure is not worth asking about again.
            busy = getattr(error, "code", 503) in (429, 502, 503, 504)
            if wait is None or not busy:
                raise
            print(f"the query service is busy ({error}); trying again in {wait} s", flush=True)
            time.sleep(wait)
    if "remark" in document and not document.get("elements"):
        raise RuntimeError(document["remark"])
    return document["elements"]


def build(layer, folder):
    selection, describe = EXTRACTS[layer]
    lines = {}
    for element in query(selection):
        point = element.get("center", element)
        if "lat" not in point or "lon" not in point:
            continue
        identity = f"{element['type'][0]}{element['id']}"
        kind, name, details = describe(element.get("tags", {}))
        pairs = "|".join(f"{label}={clean(value)}" for label, value in details if value)
        lines[identity] = (
            f"{identity}\t{point['lat']:.5f}\t{point['lon']:.5f}\t{kind}\t{clean(name)}\t{pairs}\n"
        )
    if not lines:
        raise RuntimeError(f"the query for {layer} found nothing, which cannot be right")
    today = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
    heading = f"#orion-extract\t1\t{layer}\t{today}\t{len(lines)}\n"
    # Written beside the final name and renamed, so a failed run leaves no half file.
    partial = folder / f"{layer}.tsv.partial"
    partial.write_text(heading + "".join(lines.values()), encoding="utf-8")
    partial.replace(folder / f"{layer}.tsv")
    print(f"{layer}: {len(lines)} places", flush=True)


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    folder = Path(sys.argv[1])
    folder.mkdir(parents=True, exist_ok=True)
    layers = sys.argv[2:] or list(EXTRACTS)
    unknown = [layer for layer in layers if layer not in EXTRACTS]
    if unknown:
        sys.exit(f"no such Extract: {', '.join(unknown)}; there are {', '.join(EXTRACTS)}")
    for number, layer in enumerate(layers):
        if number:
            time.sleep(PAUSE_SECONDS)
        build(layer, folder)


if __name__ == "__main__":
    main()
