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
line is one place: its OpenStreetMap id, latitude, longitude, class, name, details
as `Label=value` pairs separated by `|`, a size (a number whose meaning belongs to
the layer, such as megawatts; often empty) and, for a route such as a pipeline, its
line. A line is written the way web maps encode polylines, to four decimal places,
which keeps a point to about four characters.
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


def megawatts(text):
    """Reads a capacity such as `500 MW`, `1.2 GW` or `250 kW` as megawatts."""
    parts = text.replace(",", ".").split()
    if not parts:
        return None
    try:
        number = float(parts[0])
    except ValueError:
        return None
    unit = parts[1].lower() if len(parts) > 1 else "w"
    scale = {"gw": 1000.0, "mw": 1.0, "kw": 0.001, "w": 0.000001}.get(unit)
    return None if scale is None else number * scale


# How OpenStreetMap's words for what a plant runs on map to Orion's classes.
PLANT_FUELS = {
    "solar": (0, "Solar"), "wind": (1, "Wind"), "hydro": (2, "Hydro"),
    "gas": (3, "Gas"), "coal": (4, "Coal"), "oil": (5, "Oil"), "diesel": (5, "Oil"),
    "nuclear": (6, "Nuclear"), "biomass": (7, "Biomass or waste"),
    "biogas": (7, "Biomass or waste"), "biofuel": (7, "Biomass or waste"),
    "waste": (7, "Biomass or waste"), "geothermal": (8, "Geothermal"),
}


def power_plant(tags):
    source = first(tags, "plant:source").split(";")[0].strip().lower()
    kind, fuel = PLANT_FUELS.get(source, (9, ""))
    capacity = megawatts(first(tags, "plant:output:electricity"))
    name = first(tags, "name", "name:en") or (f"{fuel} power plant" if fuel else "Power plant")
    return kind, name, [
        ("Runs on", fuel or first(tags, "plant:source")),
        ("Capacity", f"{capacity:g} MW" if capacity else ""),
        ("Operator", first(tags, "operator", "owner")),
        ("Method", first(tags, "plant:method")),
        ("Opened", first(tags, "start_date")),
    ], capacity


# What is dug, by Orion's classes. Anything else named is "another mineral".
MINE_RESOURCES = {
    "coal": 0, "lignite": 0, "iron_ore": 1, "iron": 1, "copper": 2, "gold": 3, "silver": 3,
    "platinum": 3, "diamond": 3, "lithium": 4,
    "sand": 6, "gravel": 6, "stone": 6, "limestone": 6, "clay": 6, "granite": 6, "marble": 6,
    "basalt": 6, "sandstone": 6, "aggregate": 6, "chalk": 6, "slate": 6, "dolomite": 6,
    "gypsum": 6, "peat": 6, "rock": 6,
}


def mine(tags):
    resource = first(tags, "resource").split(";")[0].strip().lower()
    kind = MINE_RESOURCES.get(resource, 5 if resource else 7)
    what = resource.replace("_", " ")
    label = "Mine shaft" if tags.get("man_made") == "mineshaft" else "Mine or quarry"
    name = first(tags, "name", "name:en") or (f"{label}, {what}" if what else label)
    return kind, name, [
        ("Kind", label),
        ("Digs", what),
        ("Operator", first(tags, "operator", "owner")),
        ("Opened", first(tags, "start_date")),
    ]


def oil_and_gas_site(tags):
    refinery = tags.get("industrial") == "refinery"
    label = "Refinery" if refinery else "Oil or gas site"
    name = first(tags, "name", "name:en") or label
    return 0 if refinery else 1, name, [
        ("Kind", label),
        ("Operator", first(tags, "operator", "owner")),
        ("Handles", first(tags, "product", "substance", "content")),
    ]


def dam(tags):
    return 0, first(tags, "name", "name:en") or "Dam", [
        ("Height", first(tags, "height")),
        ("Operator", first(tags, "operator", "owner")),
        ("Opened", first(tags, "start_date")),
    ]


# What a pipeline carries, by Orion's classes.
PIPELINE_SUBSTANCES = {
    "gas": 0, "natural_gas": 0, "lng": 0, "cng": 0,
    "oil": 1, "crude_oil": 1, "petroleum": 1,
    "fuel": 2, "refined_products": 2, "ngl": 2, "hydrocarbons": 2,
}


def pipeline(tags):
    substance = first(tags, "substance", "type").split(";")[0].strip().lower()
    kind = PIPELINE_SUBSTANCES.get(substance, 2)
    what = substance.replace("_", " ")
    name = first(tags, "name", "name:en") or (f"Pipeline, {what}" if what else "Pipeline")
    return kind, name, [
        ("Carries", what),
        ("Operator", first(tags, "operator", "owner")),
        ("Diameter", first(tags, "diameter")),
        ("Laid", first(tags, "location")),
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
    "power_plants": (
        'nwr["power"="plant"];',
        power_plant,
    ),
    "mines": (
        'nwr["landuse"="quarry"]["resource"];nwr["man_made"="mineshaft"]["resource"];'
        'nwr["industrial"="mine"];',
        mine,
    ),
    "oil_and_gas_sites": (
        'nwr["industrial"~"^(refinery|oil|gas|oil_terminal|lng_terminal)$"];',
        oil_and_gas_site,
    ),
    "dams": (
        'nwr["waterway"="dam"]["name"];',
        dam,
    ),
    "pipelines": (
        'way["man_made"="pipeline"]'
        '["substance"~"^(gas|natural_gas|lng|cng|oil|crude_oil|petroleum|fuel|refined_products|ngl|hydrocarbons)$"];',
        pipeline,
    ),
}

# Extracts whose places are routes: their lines are fetched and kept.
ROUTES = {"pipelines"}
# How far, in degrees, a route's line may be straightened to save space: about 50 m.
ROUTE_TOLERANCE = 0.0005


def clean(text):
    """Text safe to put in one field: no tabs, line ends or the separators used inside it."""
    return " ".join(str(text).replace("|", "/").replace("=", ":").split())[:200]


def simplify(points, tolerance):
    """Drops the points a line can do without, never straying more than `tolerance`."""
    if len(points) <= 2:
        return points
    keep = [False] * len(points)
    keep[0] = keep[-1] = True
    spans = [(0, len(points) - 1)]
    while spans:
        start, end = spans.pop()
        (ax, ay), (bx, by) = points[start], points[end]
        dx, dy = bx - ax, by - ay
        length_squared = dx * dx + dy * dy
        farthest, at = 0.0, start
        for index in range(start + 1, end):
            px, py = points[index]
            t = 0.0 if length_squared == 0 else max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / length_squared))
            distance = ((px - ax - t * dx) ** 2 + (py - ay - t * dy) ** 2) ** 0.5
            if distance > farthest:
                farthest, at = distance, index
        if farthest > tolerance:
            keep[at] = True
            spans.append((start, at))
            spans.append((at, end))
    return [point for point, kept in zip(points, keep) if kept]


def encode_line(points):
    """Writes (latitude, longitude) points as an encoded polyline to four decimal places."""
    text = []
    previous = (0, 0)
    for latitude, longitude in points:
        current = (round(latitude * 10_000), round(longitude * 10_000))
        for now, before in zip(current, previous):
            value = (now - before) << 1
            if value < 0:
                value = ~value
            while value >= 0x20:
                text.append(chr((0x20 | (value & 0x1F)) + 63))
                value >>= 5
            text.append(chr(value + 63))
        previous = current
    return "".join(text)


def query(selection, with_lines=False):
    """Asks the query service for every place a selection matches, with a point for
    each, or its whole line when `with_lines` is set."""
    shape = "geom" if with_lines else "center"
    text = f"[out:json][timeout:{QUERY_SECONDS}];({selection});out {shape} tags qt;"
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
    for element in query(selection, with_lines=layer in ROUTES):
        line = ""
        point = element.get("center", element)
        if layer in ROUTES:
            route = [(node["lat"], node["lon"]) for node in element.get("geometry", []) if node]
            route = simplify(route, ROUTE_TOLERANCE)
            if len(route) < 2:
                continue
            line = encode_line(route)
            middle = route[len(route) // 2]
            point = {"lat": middle[0], "lon": middle[1]}
        if "lat" not in point or "lon" not in point:
            continue
        identity = f"{element['type'][0]}{element['id']}"
        kind, name, details, *size = describe(element.get("tags", {}))
        pairs = "|".join(f"{label}={clean(value)}" for label, value in details if value)
        size = f"{size[0]:g}" if size and size[0] else ""
        lines[identity] = (
            f"{identity}\t{point['lat']:.5f}\t{point['lon']:.5f}\t{kind}\t{clean(name)}\t{pairs}"
            f"\t{size}\t{line}\n"
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
