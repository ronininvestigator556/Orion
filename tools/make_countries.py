#!/usr/bin/env python3
"""Builds tools/countries.tsv, the table of countries that make_advisories.py places
Travel Advisories by, from Natural Earth's public-domain map of the world's countries.

Run with Python 3 and nothing else installed, on the 1:50m "Admin 0 - Countries" file
from https://www.naturalearthdata.com saved as GeoJSON:

    python3 tools/make_countries.py ne_50m_admin_0_countries.geojson tools/countries.tsv

Each line is a country or territory: its two-letter ISO code, the two-letter code the
U.S. government uses for it where that differs or is known (its advisories are filed
under those), its name in English, and the latitude and longitude of the point where
a map would put its name. The table only needs rebuilding when the world changes.
"""

import json
import sys


def main(source, target):
    with open(source, encoding="utf-8") as file:
        countries = json.load(file)["features"]
    lines = {}
    for country in countries:
        facts = country["properties"]
        iso = facts["ISO_A2_EH"]
        if len(iso) != 2 or not iso.isalpha():
            continue
        us_code = facts["FIPS_10"] if facts["FIPS_10"].isalpha() else ""
        # A country and its dependencies can share a code; the country comes first.
        lines.setdefault(
            iso,
            f"{iso}\t{us_code}\t{facts['NAME_EN']}\t{facts['LABEL_Y']:.3f}\t{facts['LABEL_X']:.3f}",
        )
    with open(target, "w", encoding="utf-8") as file:
        file.write("# Countries, from Natural Earth (public domain). Built by make_countries.py.\n")
        file.write("\n".join(lines[iso] for iso in sorted(lines)) + "\n")
    print(f"{len(lines)} countries")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
