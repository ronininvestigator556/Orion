#!/usr/bin/env python3
"""Builds the two files of Armed Conflicts and Conflict Events that Orion reads.

They come from the Uppsala Conflict Data Program's Candidate Events Dataset, which
records each incident of organised violence in which at least one person died:
https://ucdp.uu.se/downloads/ (CC BY 4.0; cite the publications listed there).

UCDP puts out one file for each month, about three weeks after the month ends, under
a name that changes with the month. This job finds the newest three, and from them
writes what every copy of Orion downloads (ADR 0004).

Run with Python 3 and nothing else installed:

    python3 tools/make_conflicts.py <folder>

It writes two files into the folder:

    conflict_events.tsv   one line for each incident that UCDP could place at least
                          to a province
    armed_conflicts.tsv   one line for each conflict in each country, with its sides
                          and the deaths UCDP estimates over the months covered

To read UCDP's files from a folder on disk and not from the network, as when trying a
change, add --from-files <folder>.

UCDP's files carry the news headlines each record was drawn from. Those can name
individuals, so they are never read here (ADR 0002).
"""

import collections
import csv
import dataclasses
import datetime
import io
import os
import re
import statistics
import sys
import urllib.error
import urllib.request

SOURCE = "https://ucdp.uu.se/downloads/candidateged"
# Says who is asking, without saying more than that.
USER_AGENT = "orion-conflicts/1"
# How many of UCDP's monthly files are joined, and how far back to look for them.
MONTHS = 3
MONTHS_TRIED = 8
VERSION = "1"
# UCDP says how exactly it could place each incident, from 1 (the very spot) to 7
# (somewhere at sea). Up to 4, a province, an incident is worth a mark of its own;
# beyond that it only counts towards its conflict's totals.
PLACED_WELL_ENOUGH = 4
KINDS = {"1": "state", "2": "groups", "3": "civilians"}
# The most pairs of sides listed for one conflict.
SIDES_LISTED = 4


@dataclasses.dataclass(frozen=True)
class Event:
    id: str
    day: str
    lat: float
    lon: float
    precision: int
    kind: str
    country: str
    place: str
    conflict_id: str
    conflict: str
    side_a: str
    side_b: str
    best: int
    low: int
    high: int
    checked: bool


@dataclasses.dataclass(frozen=True)
class Conflict:
    id: str
    lat: float
    lon: float
    kind: str
    country: str
    name: str
    sides: str
    events: int
    best: int
    low: int
    high: int


def tidy(text):
    return " ".join(str(text or "").split())


def party(name):
    """A side's name. UCDP writes XXX and a number for one it has yet to identify."""
    name = tidy(name)
    return "an unidentified armed group" if re.fullmatch(r"XXX\d+", name) else name


def conflict_name(name, country):
    """A conflict's name, with UCDP's placeholders for unidentified sides put in words."""
    name = tidy(name)
    if not re.search(r"XXX\d+", name):
        return name
    if name.endswith("- Civilians"):
        return f"Unidentified armed group against civilians in {country}"
    return f"Unidentified armed groups in {country}"


def number(text):
    try:
        return int(float(text))
    except (TypeError, ValueError):
        return 0


def read_events(text):
    """The incidents in one of UCDP's files. Only the columns needed are looked at."""
    events = []
    for row in csv.DictReader(io.StringIO(text.lstrip("﻿"))):
        try:
            lat, lon = float(row["latitude"]), float(row["longitude"])
            precision = int(row["where_prec"])
            day = datetime.date.fromisoformat(row["date_start"][:10]).isoformat()
        except (KeyError, TypeError, ValueError):
            continue
        kind = KINDS.get(row.get("type_of_violence"))
        if not kind or not row.get("id"):
            continue
        country = tidy(row.get("country"))
        events.append(
            Event(
                id=row["id"],
                day=day,
                lat=lat,
                lon=lon,
                precision=precision,
                kind=kind,
                country=country,
                place=tidy(row.get("where_coordinates")),
                conflict_id=tidy(row.get("conflict_new_id")),
                conflict=conflict_name(row.get("conflict_name"), country),
                side_a=party(row.get("side_a")),
                side_b=party(row.get("side_b")),
                best=number(row.get("best")),
                low=number(row.get("low")),
                high=number(row.get("high")),
                checked=row.get("code_status") == "Clear",
            )
        )
    return events


def conflicts(events):
    """One line for each conflict in each country, deadliest first."""
    gathered = collections.defaultdict(list)
    for event in events:
        gathered[(event.conflict_id, event.country)].append(event)
    result = []
    for (conflict_id, country), mine in gathered.items():
        # It is marked in the middle of where its incidents were, going by those
        # that could be placed, so one stray report does not drag it away.
        placed = [event for event in mine if event.precision <= PLACED_WELL_ENOUGH] or mine
        pairs = collections.Counter(f"{event.side_a} – {event.side_b}" for event in mine)
        sides = [pair for pair, _ in pairs.most_common(SIDES_LISTED)]
        if len(pairs) > SIDES_LISTED:
            sides.append(f"and {len(pairs) - SIDES_LISTED} more")
        result.append(
            Conflict(
                id=f"{conflict_id}/{country}",
                lat=statistics.median(event.lat for event in placed),
                lon=statistics.median(event.lon for event in placed),
                kind=collections.Counter(event.kind for event in mine).most_common(1)[0][0],
                country=country,
                name=mine[0].conflict,
                sides="; ".join(sides),
                events=len(mine),
                best=sum(event.best for event in mine),
                low=sum(event.low for event in mine),
                high=sum(event.high for event in mine),
            )
        )
    result.sort(key=lambda conflict: (-conflict.best, conflict.name, conflict.country))
    return result


def file_names(today):
    """The names UCDP's monthly files would have, newest month first."""
    names, year, month = [], today.year, today.month
    for _ in range(MONTHS_TRIED):
        month -= 1
        if month == 0:
            year, month = year - 1, 12
        names.append(f"GEDEvent_v{year % 100}_0_{month}.csv")
    return names


def month_of(name):
    """The first day of the month one of UCDP's monthly files is for."""
    year, month = re.fullmatch(r"GEDEvent_v(\d+)_0_(\d+)\.csv", name).groups()
    return datetime.date(2000 + int(year), int(month), 1)


def period(used, events):
    """The days covered: from the start of the oldest month read to the last incident.

    A few records in each file are of incidents from long before, found late. They
    are kept, but the period is the months the files are for.
    """
    first = min(month_of(name) for name in used).isoformat()
    last = max((event.day for event in events), default=first)
    return first, last


def heading(kind, built, count, covered):
    return f"#orion-{kind}\t{VERSION}\t{built}\t{count}\t{covered[0]}\t{covered[1]}\n"


def write_events(path, built, events, covered):
    """Writes the incidents worth a mark of their own, newest first."""
    shown = [event for event in events if event.precision <= PLACED_WELL_ENOUGH]
    shown.sort(key=lambda event: (event.day, event.id), reverse=True)
    with open(path, "w", encoding="utf-8") as file:
        file.write(heading("conflict-events", built, len(shown), covered))
        for event in shown:
            fields = [
                event.id,
                event.day,
                f"{event.lat:.5f}",
                f"{event.lon:.5f}",
                event.precision,
                event.kind,
                event.country,
                event.place,
                event.conflict,
                event.side_a,
                event.side_b,
                event.best,
                event.low,
                event.high,
                1 if event.checked else 0,
            ]
            file.write("\t".join(str(field) for field in fields) + "\n")
    return len(shown)


def write_conflicts(path, built, found, covered):
    with open(path, "w", encoding="utf-8") as file:
        file.write(heading("armed-conflicts", built, len(found), covered))
        for conflict in found:
            fields = dataclasses.astuple(conflict)
            file.write(
                "\t".join(f"{field:.4f}" if isinstance(field, float) else str(field) for field in fields)
                + "\n"
            )


def fetch(name, from_files):
    """One of UCDP's files as text, or None if there is no file of that name."""
    if from_files:
        try:
            with open(os.path.join(from_files, name), encoding="utf-8-sig") as file:
                return file.read()
        except FileNotFoundError:
            return None
    request = urllib.request.Request(f"{SOURCE}/{name}", headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            return response.read().decode("utf-8-sig")
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise


def main(arguments):
    from_files = None
    if "--from-files" in arguments:
        at = arguments.index("--from-files")
        from_files = arguments[at + 1]
        del arguments[at : at + 2]
    if len(arguments) != 1:
        sys.exit(__doc__)
    folder = arguments[0]
    os.makedirs(folder, exist_ok=True)
    now = datetime.datetime.now(datetime.timezone.utc)

    by_id, used = {}, []
    for name in file_names(now.date()):
        text = fetch(name, from_files)
        if text is None:
            continue
        used.append(name)
        # A later file may correct an earlier one, and the newest was read first.
        for event in read_events(text):
            by_id.setdefault(event.id, event)
        if len(used) == MONTHS:
            break
    if not by_id:
        sys.exit("none of UCDP's monthly files could be found or read")

    events = list(by_id.values())
    found = conflicts(events)
    built = now.strftime("%Y-%m-%dT%H:%M:%SZ")
    covered = period(used, events)
    shown = write_events(os.path.join(folder, "conflict_events.tsv"), built, events, covered)
    write_conflicts(os.path.join(folder, "armed_conflicts.tsv"), built, found, covered)
    print(f"read {', '.join(used)}")
    print(
        f"{len(events)} incidents, {shown} placed well enough to mark; "
        f"{len(found)} conflicts; {sum(event.best for event in events)} deaths estimated"
    )


if __name__ == "__main__":
    main(sys.argv[1:])
