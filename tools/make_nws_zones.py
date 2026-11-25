"""Builds the table of U.S. forecast zones bundled for the weather alerts Layer.

Most alerts from the National Weather Service name the zones they cover instead of
carrying an outline, so Orion needs to know where each zone is. This reads the
zone files the Weather Service publishes, which are in the public domain, and writes
one line per zone: its code, and the latitude and longitude of its middle.

Download the current files linked from these pages into one folder:

    https://www.weather.gov/gis/PublicZones    (z_*.zip)
    https://www.weather.gov/gis/Counties       (c_*.zip)
    https://www.weather.gov/gis/MarineZones    (mz*.zip and oz*.zip)

Then run, from the repository root, with Python 3 and nothing else installed:

    python3 tools/make_nws_zones.py <folder> crates/orion-data/data/nws_zones.tsv
"""

import io
import struct
import sys
import zipfile
from pathlib import Path


def records(dbf: bytes):
    """Yields each row of a dBASE table as a dict of field name to stripped text."""
    count, header_size, record_size = struct.unpack_from("<IHH", dbf, 4)
    fields = []
    offset = 32
    while dbf[offset] != 0x0D:
        name = dbf[offset : offset + 11].split(b"\0", 1)[0].decode("ascii")
        fields.append((name, dbf[offset + 16]))
        offset += 32
    for index in range(count):
        start = header_size + index * record_size
        if dbf[start : start + 1] == b"*":  # deleted
            continue
        position = start + 1
        row = {}
        for name, length in fields:
            row[name] = dbf[position : position + length].decode("latin-1").strip()
            position += length
        yield row


def table(archive: Path):
    with zipfile.ZipFile(archive) as files:
        name = next(name for name in files.namelist() if name.lower().endswith(".dbf"))
        return list(records(files.read(name)))


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    folder, destination = Path(sys.argv[1]), Path(sys.argv[2])
    zones = {}

    def add(code, row):
        try:
            latitude, longitude = float(row["LAT"]), float(row["LON"])
        except (KeyError, ValueError):
            return
        # A zone split into several pieces is listed once per piece: keep the first.
        zones.setdefault(code, (latitude, longitude))

    for archive in sorted(folder.glob("*.zip")):
        kind = archive.name.lower()
        rows = table(archive)
        for row in rows:
            if kind.startswith("z_"):
                add(f"{row['STATE']}Z{row['ZONE']}", row)
            elif kind.startswith("c_"):
                add(f"{row['STATE']}C{row['FIPS'][2:]}", row)
            elif kind.startswith(("mz", "oz")):
                add(row["ID"], row)
        print(f"{archive.name}: {len(rows)} rows")

    lines = [f"{code}\t{lat:.3f}\t{lon:.3f}\n" for code, (lat, lon) in sorted(zones.items())]
    destination.write_text("".join(lines), encoding="ascii")
    print(f"{len(lines)} zones written to {destination}")


if __name__ == "__main__":
    main()
