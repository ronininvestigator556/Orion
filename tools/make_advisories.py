#!/usr/bin/env python3
"""Joins three governments' Travel Advisories into the one file that Orion reads.

The United States, the United Kingdom and Canada each publish their advice about
travelling to every country, for anyone to reuse:

  - U.S. Department of State: public domain. A level from 1 to 4 for each country.
  - UK Foreign, Commonwealth & Development Office, through GOV.UK: Open Government
    Licence v3.0. What last changed in its advice for each country.
  - Global Affairs Canada: Open Government Licence - Canada. A level for each country.

Each files its advice under its own names and codes for countries, so every copy of
Orion would have to carry the tables to join them. This job joins them once and
publishes the result (ADR 0004).

Run with Python 3 and nothing else installed:

    python3 tools/make_advisories.py <folder>

It writes <folder>/advisories.tsv: a heading, then one line for each country that at
least one of the three has advice for. Countries are placed by the table Orion itself
carries, crates/orion-data/data/countries.tsv.
To read the three lists from files on disk named us.json, uk.json and ca.json, as when
trying a change, add --from-files <folder>.
"""

import dataclasses
import datetime
import html
import json
import os
import re
import sys
import unicodedata
import urllib.request

SOURCES = {
    "us": "https://cadataapi.state.gov/api/TravelAdvisories",
    "uk": "https://www.gov.uk/api/content/foreign-travel-advice",
    "ca": "https://data.international.gc.ca/travel-voyage/index-alpha-eng.json",
}
# Says who is asking, without saying more than that.
USER_AGENT = "orion-advisories/1"
HEADING = "#orion-advisories"
VERSION = "1"
# The most of a government's summary that is kept, cut at the end of a sentence.
SUMMARY_LETTERS = 600

# Names a government uses that the table of countries does not, as the table has them.
ALSO_KNOWN_AS = {
    "burma": "myanmar",
    "burma myanmar": "myanmar",
    "myanmar burma": "myanmar",
    "cote d ivoire": "ivory coast",
    "cabo verde": "cape verde",
    "czechia": "czech republic",
    "congo": "republic of the congo",
    "congo brazzaville": "republic of the congo",
    "republic of congo": "republic of the congo",
    "congo kinshasa": "democratic republic of the congo",
    "congo democratic republic": "democratic republic of the congo",
    "democratic republic of congo": "democratic republic of the congo",
    "dr congo": "democratic republic of the congo",
    "korea south": "south korea",
    "korea north": "north korea",
    "republic of korea": "south korea",
    "democratic people s republic of korea": "north korea",
    "china": "people s republic of china",
    "mainland china": "people s republic of china",
    "gambia": "the gambia",
    "bahamas": "the bahamas",
    "timor leste": "east timor",
    "usa": "united states of america",
    "united states": "united states of america",
    "turkiye": "turkey",
    "micronesia": "federated states of micronesia",
    "sao tome and principe": "sao tome and principe",
    "st martin": "saint martin",
    "vatican city": "vatican city",
    "holy see": "vatican city",
    "kyrgyz republic": "kyrgyzstan",
    "swaziland": "eswatini",
    "macao": "macau",
    "the occupied palestinian territories": "palestine",
    "israel the west bank and gaza": "israel",
    "israel and palestine": "israel",
    "west bank and gaza": "palestine",
    "st maarten": "sint maarten",
    "falkland islands": "falkland islands",
}


@dataclasses.dataclass(frozen=True)
class Country:
    iso: str
    us_code: str
    name: str
    lat: str
    lon: str


@dataclasses.dataclass(frozen=True)
class Advice:
    """What one government says about one country."""

    advice: str
    updated: str
    url: str
    level: int | None = None
    summary: str = ""


class Countries:
    """The table of countries, findable by any of the ways a government names one."""

    def __init__(self, countries):
        self.all = countries
        self.by_iso = {country.iso: country for country in countries}
        self.by_us_code = {country.us_code: country for country in countries if country.us_code}
        self.by_name = {plain_name(country.name): country for country in countries}

    def named(self, name):
        return self.by_name.get(known_name(name))


def read_countries(text):
    return Countries(
        [Country(*line.split("\t")) for line in text.splitlines() if line and not line.startswith("#")]
    )


def plain_name(name):
    """A country's name with everything that varies between lists taken out."""
    letters = unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode().lower()
    letters = letters.replace("&", " and ")
    words = re.sub(r"[^a-z]+", " ", letters).split()
    words = ["saint" if word == "st" else word for word in words]
    if words[:1] == ["the"]:
        words = words[1:]
    return " ".join(words)


def known_name(name):
    """A country's plain name as the table of countries has it."""
    plain = plain_name(name)
    return plain_name(ALSO_KNOWN_AS.get(plain, plain))


def plain_text(markup, limit=SUMMARY_LETTERS):
    """Text without its markup, cut at the end of a sentence if it runs long."""
    text = html.unescape(re.sub(r"<[^>]+>", " ", markup))
    text = re.sub(r"\s+", " ", text).strip()
    text = re.sub(r"\s+([.,;:])", r"\1", text)
    if len(text) <= limit:
        return text
    cut = text[:limit]
    end = max(cut.rfind(". "), cut.rfind("! "), cut.rfind("? "))
    return cut[: end + 1] if end > 0 else cut.rstrip() + "…"


def day(moment):
    return moment[:10]


def from_united_states(text, countries):
    """Returns the advice by country, and the places the table could not place."""
    found, unplaced = {}, []
    for entry in json.loads(text):
        title = re.match(r"^(.*?) - Level (\d): (.*)$", entry["Title"])
        if not title:
            continue
        name, level, advice = title.groups()
        codes = entry.get("Category") or []
        if isinstance(codes, str):
            codes = re.findall(r"[A-Z]{2}", codes)
        country = countries.named(name)
        for code in codes:
            country = country or countries.by_us_code.get(code)
        if not country:
            unplaced.append(name)
            continue
        found[country.iso] = Advice(
            advice=advice.strip(),
            updated=day(entry.get("Updated", "")),
            url=entry.get("Link", ""),
            level=int(level),
            summary=plain_text(entry.get("Summary", "")),
        )
    return found, unplaced


def from_united_kingdom(text, countries):
    found, unplaced = {}, []
    for entry in json.loads(text)["links"]["children"]:
        name = entry["details"]["country"]["name"]
        country = countries.named(name)
        if not country:
            unplaced.append(name)
            continue
        found[country.iso] = Advice(
            advice=plain_text(entry["details"].get("change_description", ""), limit=300),
            updated=day(entry.get("public_updated_at", "")),
            url=entry.get("web_url", ""),
        )
    return found, unplaced


def from_canada(text, countries):
    found, unplaced = {}, []
    for iso, entry in json.loads(text)["data"].items():
        country = countries.by_iso.get(iso)
        if not country:
            unplaced.append(iso)
            continue
        english = entry.get("eng", {})
        advice = english.get("advisory-text", "")
        if entry.get("has-regional-advisory"):
            advice += " (some regions differ)"
        found[iso] = Advice(
            advice=advice,
            updated=day(entry.get("date-published", {}).get("date", "")),
            url=f"https://travel.gc.ca/destinations/{english.get('url-slug', '')}",
            level=entry.get("advisory-state"),
        )
    return found, unplaced


def tidy(field):
    return " ".join(str(field).split())


def write(path, built, countries, us, uk, canada):
    """Writes a line for each country any of the three has advice for. Returns how many."""
    lines = []
    for country in countries.all:
        mine = [us.get(country.iso), uk.get(country.iso), canada.get(country.iso)]
        if not any(mine):
            continue
        american, british, canadian = (one or Advice("", "", "") for one in mine)
        fields = [
            country.iso,
            country.name,
            country.lat,
            country.lon,
            "" if american.level is None else american.level,
            american.advice,
            american.updated,
            american.url,
            american.summary,
            british.advice,
            british.updated,
            british.url,
            canadian.advice,
            canadian.updated,
            canadian.url,
        ]
        lines.append("\t".join(tidy(field) for field in fields))
    with open(path, "w", encoding="utf-8") as file:
        file.write(f"{HEADING}\t{VERSION}\t{built}\t{len(lines)}\n")
        file.write("".join(line + "\n" for line in lines))
    return len(lines)


def fetch(name, from_files):
    if from_files:
        with open(os.path.join(from_files, f"{name}.json"), encoding="utf-8") as file:
            return file.read()
    request = urllib.request.Request(SOURCES[name], headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read().decode("utf-8")


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
    here = os.path.dirname(os.path.abspath(__file__))
    table = os.path.join(here, "..", "crates", "orion-data", "data", "countries.tsv")
    with open(table, encoding="utf-8") as file:
        countries = read_countries(file.read())

    us, no_us = from_united_states(fetch("us", from_files), countries)
    uk, no_uk = from_united_kingdom(fetch("uk", from_files), countries)
    canada, no_canada = from_canada(fetch("ca", from_files), countries)
    # A list that has shrunk to almost nothing has changed its shape, and publishing
    # what is left would wipe the Layer.
    for name, found in (("United States", us), ("United Kingdom", uk), ("Canada", canada)):
        if len(found) < 100:
            sys.exit(f"only {len(found)} countries could be read from the {name}'s list")

    built = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    count = write(os.path.join(folder, "advisories.tsv"), built, countries, us, uk, canada)
    print(f"{count} countries: {len(us)} from the United States, {len(uk)} from the United Kingdom, {len(canada)} from Canada")
    for name, unplaced in (("United States", no_us), ("United Kingdom", no_uk), ("Canada", no_canada)):
        if unplaced:
            print(f"  not placed from the {name}: {', '.join(sorted(unplaced))}")


if __name__ == "__main__":
    main(sys.argv[1:])
