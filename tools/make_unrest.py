#!/usr/bin/env python3
"""Sifts GDELT's index of the world's news into the file of Unrest Reports that Orion reads.

GDELT (https://www.gdeltproject.org) reads the news by machine and publishes, every
quarter of an hour, two files of the events it believes the articles describe: one
from articles in English and one from articles it translated from other languages. Its terms
allow the files to be republished with credit. Each is far too large and too rough
for every copy of Orion to sift for itself, so this job does it once an hour and
publishes the result (ADR 0004).

Run with Python 3 and nothing else installed:

    python3 tools/make_unrest.py <folder>

The folder holds two files, both rewritten on every run:

    unrest_seen.tsv   the job's own memory: every article seen placing a protest in a
                      town in the past week, and the last file of GDELT's it read
    unrest.tsv        what Orion reads: one line for each town and day

If unrest_seen.tsv is not there the job starts afresh from the past three days. To
read GDELT's files from a folder on disk and not from the network, as when trying a
change, add --from-files <folder>.

What is kept, and why, is the whole of the job, so it is all in `sift` and `reports`.
GDELT is often wrong: it takes a story about a bakery for a march, and a paragraph
that mentions Moscow for a protest there. Tried on two days of its files in November
2026, three in four of the towns it named had no protest. Two checks made most of
the difference, and both are applied here:

  - The article's own address must name unrest. News sites build an address from the
    headline, so "pensioners-protest-today-over-arrears" is an article about a
    protest and "washington-pushes-nato" is not. This loses outlets whose addresses
    are only numbers, and most that do not write in Latin letters.
  - GDELT must place it in a town, not merely a country or a state, and say that it
    happened on the day the article appeared or the day before.

What kind it was is read from the same words. How many different outlets carried it
is counted, and Orion shows by default only what at least two did.

No name of any person or organisation is taken from GDELT: only where, when, what
kind and which articles (ADR 0002).
"""

import dataclasses
import datetime
import io
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile

SOURCE = "https://data.gdeltproject.org/gdeltv2"
# Says who is asking, without saying more than that.
USER_AGENT = "orion-unrest/1"
QUARTER_HOUR = datetime.timedelta(minutes=15)
# How far back a first run reaches, and the most any run will try to catch up on.
FIRST_RUN_DAYS = 3
CATCH_UP_DAYS = 7
# How long a report is kept after the day it is about.
KEPT_DAYS = 7
# How many articles a report names, one from each outlet.
ARTICLES_LISTED = 5
# The fewest words an address must have to count as made from a headline.
HEADLINE_WORDS = 3
# A short rest between files, so the Source is not hammered when catching up.
PAUSE_SECONDS = 0.25

# Where the columns this job reads sit in a line of GDELT's event file, which has no
# heading. See http://data.gdeltproject.org/documentation/GDELT-Event_Codebook-V2.0.pdf
COLUMNS = 61
DAY, ROOT_CODE = 1, 28
GEO_TYPE, PLACE, LATITUDE, LONGITUDE, PLACE_ID = 51, 52, 56, 57, 58
ADDED, ARTICLE = 59, 60
PROTEST_ROOT = "14"
# A town in the United States, or a town anywhere else. The other types are whole
# countries, states and regions, which say nothing useful about where.
TOWNS = ("3", "4")

# The words that, in an article's address, say what it is about. English first, then
# the commonest in French, Spanish, German, Italian and the Indian press.
VIOLENT_WORDS = frozenset(
    "riot riots rioters rioting clash clashes clashed teargas looting looters stoned "
    "torched emeute emeutes".split()
)
STRIKE_WORDS = frozenset(
    "strike strikes strikers striking walkout walkouts greve greves huelga huelgas "
    "streik streiks warnstreik sciopero scioperi staking strajk bandh hartal".split()
)
PROTEST_WORDS = frozenset(
    "protest protests protester protesters protestor protestors protesting protested "
    "demonstration demonstrations demonstrator demonstrators demonstrate demonstrates "
    "demonstrated rally rallies march marches marchers marched unrest blockade "
    "blockades picket pickets manifestation manifestations manifestants manif "
    "protesta protestas protesto protestos proteste protesty marcha marchas "
    "manifestacion manifestaciones manifestacao manifestazione manifestazioni "
    "rassemblement blocage blocages cortege kundgebung betoging dharna agitation "
    "agitating".split()
)
# A strike of this sort is not people stopping work.
NOT_A_WALKOUT = frozenset(
    "air drone missile military nuclear lightning hunger israeli russian american us "
    "would could first preemptive".split()
)

SEEN_HEADING = "#orion-unrest-seen"
REPORTS_HEADING = "#orion-unrest"
VERSION = "1"


@dataclasses.dataclass(frozen=True)
class Seen:
    """One article placing one kind of protest in one town on one day."""

    day: str
    kind: str
    key: str
    lat: float
    lon: float
    place: str
    outlet: str
    url: str
    added: str


@dataclasses.dataclass(frozen=True)
class Report:
    """What the articles together say about one town on one day."""

    id: str
    day: str
    lat: float
    lon: float
    kind: str
    place: str
    outlets: int
    urls: list


def moment(slot):
    return datetime.datetime.strptime(slot, "%Y%m%d%H%M%S")


def written(slot_moment):
    return slot_moment.strftime("%Y%m%d%H%M%S")


def sift(text):
    """Returns what is worth keeping from one of GDELT's event files."""
    kept = []
    for line in text.splitlines():
        columns = line.split("\t")
        if len(columns) != COLUMNS or columns[ROOT_CODE] != PROTEST_ROOT:
            continue
        if columns[GEO_TYPE] not in TOWNS:
            continue
        url = columns[ARTICLE].strip()
        outlet = urllib.parse.urlsplit(url).netloc.lower().removeprefix("www.")
        if not outlet or any(character.isspace() for character in url):
            continue
        kind = kind_named(url)
        if kind is None:
            continue
        try:
            lat, lon = float(columns[LATITUDE]), float(columns[LONGITUDE])
            day = datetime.datetime.strptime(columns[DAY], "%Y%m%d").date()
            added = moment(columns[ADDED]).date()
        except ValueError:
            continue
        # News about an old protest, such as its trial years later, is not unrest now.
        if not datetime.timedelta(0) <= added - day <= datetime.timedelta(days=1):
            continue
        kept.append(
            Seen(
                day=day.isoformat(),
                kind=kind,
                key=columns[PLACE_ID].strip() or f"{lat},{lon}",
                lat=lat,
                lon=lon,
                place=" ".join(columns[PLACE].split()),
                outlet=outlet,
                url=url,
                added=columns[ADDED],
            )
        )
    return kept


def kind_named(url):
    """What kind of unrest an article's address names, or None if it names none.

    Only the part of the address made from the headline is read: the longest piece
    between slashes. The folders above it hold dates, and a month is no march.
    """
    path = urllib.parse.unquote(urllib.parse.urlsplit(url).path).lower()
    headline = max(path.split("/"), key=len)
    for accented, plain in (("è", "e"), ("é", "e"), ("ê", "e")):
        headline = headline.replace(accented, plain)
    for accented, plain in (("ã", "a"), ("ç", "c"), ("ó", "o")):
        headline = headline.replace(accented, plain)
    words = "".join(letter if letter.isalpha() else " " for letter in headline).split()
    # A headline has several words. A page listing everything filed under "protests"
    # has one.
    if len(words) < HEADLINE_WORDS:
        return None
    found = set()
    for at, word in enumerate(words):
        before = words[at - 1] if at else ""
        after = words[at + 1] if at + 1 < len(words) else ""
        if word in VIOLENT_WORDS or (word == "tear" and after == "gas"):
            found.add("violent")
        elif word in STRIKE_WORDS:
            if before not in NOT_A_WALKOUT:
                found.add("strike")
        elif word == "walk" and after in ("out", "outs"):
            found.add("strike")
        elif word in PROTEST_WORDS:
            # A protest petition is a court filing, and a government that protests
            # to another has sent a letter.
            if after != "petition" and not (word == "protests" and after == "to"):
                found.add("protest")
    for kind in ("violent", "strike", "protest"):
        if kind in found:
            return kind
    return None


def kind_of(outlets_by_kind, outlets):
    """Settles what kind a report is from what each outlet called it.

    One outlet's word for violence is not taken over others': a headline may say
    "clash" of an argument.
    """
    violent = outlets_by_kind.get("violent", set())
    strike = outlets_by_kind.get("strike", set())
    if len(violent) >= 2 or len(violent) == len(outlets):
        return "violent"
    if len(strike) * 2 > len(outlets):
        return "strike"
    return "protest"


def reports(seen):
    """Gathers what was seen into one report for each town and day, newest first."""
    gathered = {}
    for one in seen:
        gathered.setdefault((one.day, one.key), []).append(one)
    result = []
    for (day, key), articles in gathered.items():
        outlets, outlets_by_kind, urls = [], {}, []
        for one in articles:
            outlets_by_kind.setdefault(one.kind, set()).add(one.outlet)
            if one.outlet not in outlets:
                outlets.append(one.outlet)
                urls.append(one.url)
        first = articles[0]
        result.append(
            Report(
                id=f"{day}/{key}",
                day=day,
                lat=first.lat,
                lon=first.lon,
                kind=kind_of(outlets_by_kind, outlets),
                place=first.place,
                outlets=len(outlets),
                urls=urls[:ARTICLES_LISTED],
            )
        )
    result.sort(key=lambda report: (report.day, report.outlets), reverse=True)
    return result


def slots_after(last, latest):
    """Every quarter-hour file after `last`, up to and including `latest`."""
    slots, at, end = [], moment(last) + QUARTER_HOUR, moment(latest)
    while at <= end:
        slots.append(written(at))
        at += QUARTER_HOUR
    return slots


def slots_to_fetch(last, latest):
    """The files this run should read, given the last one the run before it read."""
    end = moment(latest)
    if last is None:
        start = end - datetime.timedelta(days=FIRST_RUN_DAYS)
    else:
        start = max(moment(last), end - datetime.timedelta(days=CATCH_UP_DAYS))
    return slots_after(written(start), latest)


def forget_old(seen, latest):
    """Drops what is about a day too long ago to be kept."""
    today = moment(latest).date()
    oldest = (today - datetime.timedelta(days=KEPT_DAYS)).isoformat()
    return [one for one in seen if one.day >= oldest]


def read_seen(path):
    """Returns the last file read and everything seen, or nothing if there is no memory."""
    try:
        with open(path, encoding="utf-8") as file:
            lines = file.read().splitlines()
    except FileNotFoundError:
        return None, []
    heading = lines[0].split("\t") if lines else []
    if heading[:2] != [SEEN_HEADING, VERSION] or len(heading) != 3:
        sys.exit(f"{path} is not this job's memory, or is from another version of it")
    seen = []
    for line in lines[1:]:
        day, kind, key, lat, lon, place, outlet, url, added = line.split("\t")
        seen.append(Seen(day, kind, key, float(lat), float(lon), place, outlet, url, added))
    return heading[2], seen


def write_seen(path, last, seen):
    with open(path, "w", encoding="utf-8") as file:
        file.write(f"{SEEN_HEADING}\t{VERSION}\t{last}\n")
        for one in seen:
            fields = dataclasses.astuple(one)
            file.write("\t".join(repr(field) if isinstance(field, float) else field for field in fields))
            file.write("\n")


def write_reports(path, built, found):
    with open(path, "w", encoding="utf-8") as file:
        file.write(f"{REPORTS_HEADING}\t{VERSION}\t{built}\t{len(found)}\n")
        for report in found:
            file.write(
                "\t".join(
                    [
                        report.id,
                        report.day,
                        f"{report.lat:.4f}",
                        f"{report.lon:.4f}",
                        report.kind,
                        report.place,
                        str(report.outlets),
                        " ".join(report.urls),
                    ]
                )
            )
            file.write("\n")


def events_text(data):
    """The event file inside one download, or nothing if the download is not one.

    GDELT now and then lists a file it has not finished writing, or skips a quarter
    of an hour altogether.
    """
    try:
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            return archive.read(archive.namelist()[0]).decode("utf-8", "replace")
    except (zipfile.BadZipFile, IndexError, KeyError):
        return ""


def get(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def latest_slot(from_files):
    if from_files:
        names = sorted(name for name in os.listdir(from_files) if name.endswith(".export.CSV.zip"))
        return names[-1].split(".")[0]
    # The first line names the newest event file: its size, a checksum and its address.
    first = get(f"{SOURCE}/lastupdate.txt").decode().splitlines()[0]
    return first.rsplit("/", 1)[-1].split(".")[0]


def file_names(slot):
    """The two files of one quarter of an hour: from English, and translated."""
    return [f"{slot}.export.CSV.zip", f"{slot}.translation.export.CSV.zip"]


def download(name, from_files):
    if from_files:
        with open(os.path.join(from_files, name), "rb") as file:
            return file.read()
    time.sleep(PAUSE_SECONDS)
    return get(f"{SOURCE}/{name}")


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
    seen_path = os.path.join(folder, "unrest_seen.tsv")

    last, seen = read_seen(seen_path)
    latest = latest_slot(from_files)
    slots = slots_to_fetch(last, latest)
    read = missing = 0
    for slot in slots:
        for name in file_names(slot):
            try:
                text = events_text(download(name, from_files))
            except (OSError, urllib.error.URLError):
                text = ""
            if text:
                read += 1
                seen.extend(sift(text))
            else:
                missing += 1
    if slots and not read:
        sys.exit(f"none of the {2 * len(slots)} files since the last run could be read")

    # The same article is often listed many times over.
    seen = forget_old(list(dict.fromkeys(seen)), latest)
    found = reports(seen)
    built = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    write_seen(seen_path, latest, seen)
    write_reports(os.path.join(folder, "unrest.tsv"), built, found)
    twice = sum(1 for report in found if report.outlets >= 2)
    print(
        f"read {read} files up to {latest} ({missing} missing); "
        f"{len(found)} towns and days, {twice} carried by two outlets or more"
    )


if __name__ == "__main__":
    main(sys.argv[1:])
