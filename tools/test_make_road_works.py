"""Tests for make_road_works.py. Run with: python3 -m unittest discover tools"""

import datetime
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import make_road_works as works  # noqa: E402

NOW = datetime.datetime(2026, 11, 25, 21, 0, tzinfo=datetime.timezone.utc)
OPEN = "https://creativecommons.org/publicdomain/zero/1.0/"


def zone(
    event_id="d09c8972",
    impact="all-lanes-closed",
    start="2025-11-05T03:00:00+00:00",
    end="2027-11-15T11:30:00+00:00",
    event_type="work-zone",
    geometry=None,
):
    return {
        "id": event_id,
        "type": "Feature",
        "geometry": geometry
        or {"type": "LineString", "coordinates": [[-87.99, 43.03], [-87.98, 43.031], [-87.97, 43.04]]},
        "properties": {
            "core_details": {
                "event_type": event_type,
                "road_names": ["I-94 EB"],
                "direction": "eastbound",
                "description": "Ramp Full Closure on I-94 EB\tat OFF RAMP\nTO HAWLEY RD.",
            },
            "start_date": start,
            "end_date": end,
            "vehicle_impact": impact,
        },
    }


def feed(features, licence=OPEN):
    info = {"publisher": "Work Zone Manager", "version": "4.2"}
    if licence:
        info["license"] = licence
    return json.dumps({"feed_info": info, "type": "FeatureCollection", "features": features})


class Reading(unittest.TestCase):
    def read(self, text):
        return works.work_zones(text, "wisconsin", "wisdot", "Wisconsin DOT", NOW)

    def test_a_work_zone_under_way_is_kept_with_where_and_what(self):
        found = self.read(feed([zone()]))
        self.assertEqual(len(found), 1)
        one = found[0]
        self.assertEqual(one.id, "wisdot/d09c8972")
        self.assertEqual(one.kind, "closure")
        # It is marked at the middle of its line.
        self.assertEqual((one.lat, one.lon), (43.031, -87.98))
        self.assertEqual(one.state, "Wisconsin")
        self.assertEqual(one.road, "I-94 EB")
        self.assertEqual(one.direction, "eastbound")
        # Tabs and line breaks would break the file it goes into.
        self.assertEqual(one.description, "Ramp Full Closure on I-94 EB at OFF RAMP TO HAWLEY RD.")
        self.assertEqual(one.publisher, "Wisconsin DOT")

    def test_a_road_is_closed_only_when_every_lane_is(self):
        kinds = [
            self.read(feed([zone(impact=impact)]))[0].kind
            for impact in ("all-lanes-closed", "some-lanes-closed", "flagging", None)
        ]
        self.assertEqual(kinds, ["closure", "roadworks", "roadworks", "roadworks"])

    def test_what_is_not_under_way_or_not_a_work_zone_is_left_out(self):
        left_out = [
            zone(start="2026-12-01T00:00:00Z"),  # not begun
            zone(end="2026-11-24T00:00:00Z"),  # over
            zone(event_type="detour"),  # the way round one, not a work zone
            zone(geometry={"type": "Polygon", "coordinates": []}),  # nowhere to mark
            zone(start="soon"),  # no telling when
        ]
        self.assertEqual(self.read(feed(left_out)), [])
        # One with no end is taken as still going on.
        self.assertEqual(len(self.read(feed([zone(end=None)]))), 1)

    def test_a_point_and_an_older_feed_are_read_too(self):
        point = zone(geometry={"type": "MultiPoint", "coordinates": [[-93.6, 41.6], [-93.5, 41.7]]})
        self.assertEqual((self.read(feed([point]))[0].lat, self.read(feed([point]))[0].lon), (41.6, -93.6))
        # Before version 4 the same facts sat one level up, under other names.
        older = {
            "type": "Feature",
            "geometry": {"type": "LineString", "coordinates": [[-93.6, 41.6], [-93.5, 41.7]]},
            "properties": {
                "road_event_id": "77",
                "event_type": "work-zone",
                "road_name": "US 30",
                "direction": "westbound",
                "description": "Bridge painting",
                "start_date": "2026-11-01T12:00:00Z",
                "vehicle_impact": "some-lanes-closed",
            },
        }
        text = json.dumps({"road_event_feed_info": {"license": OPEN}, "features": [older]})
        found = self.read(text)
        self.assertEqual(found[0].id, "wisdot/77")
        self.assertEqual(found[0].road, "US 30")

    def test_a_feed_that_states_no_open_licence_is_not_used(self):
        with self.assertRaises(works.NotOpen):
            self.read(feed([zone()], licence=None))
        with self.assertRaises(works.NotOpen):
            self.read(feed([zone()], licence="All rights reserved"))


class Registry(unittest.TestCase):
    def test_only_feeds_that_need_no_key_are_asked(self):
        registry = json.dumps(
            [
                {
                    "state": "wisconsin",
                    "issuingorganization": "Wisconsin DOT",
                    "feedname": "wisdot",
                    "url": {"url": "https://511wi.example/wzdx"},
                    "format": "geojson",
                    "active": True,
                    "needapikey": False,
                },
                {
                    "state": "colorado",
                    "issuingorganization": "Colorado DOT",
                    "feedname": "cdot",
                    "url": {"url": "https://data.cotrip.example/wzdx?apiKey="},
                    "format": "geojson",
                    "active": True,
                    "needapikey": True,
                },
                {
                    "state": "ohio",
                    "feedname": "odot",
                    "url": {"url": "https://ohio.example/wzdx"},
                    "format": "geojson",
                    "active": False,
                    "needapikey": False,
                },
            ]
        )
        self.assertEqual(
            works.keyless_feeds(registry),
            [("wisconsin", "wisdot", "Wisconsin DOT", "https://511wi.example/wzdx")],
        )


class Writing(unittest.TestCase):
    def test_the_file_the_app_reads(self):
        found = works.work_zones(feed([zone()]), "wisconsin", "wisdot", "Wisconsin DOT", NOW)
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "road_works_us.tsv")
            works.write(path, "2026-11-25T21:00:00Z", found)
            with open(path, encoding="utf-8") as file:
                lines = file.read().splitlines()
        self.assertEqual(lines[0], "#orion-road-works\t1\t2026-11-25T21:00:00Z\t1")
        self.assertEqual(
            lines[1].split("\t"),
            [
                "wisdot/d09c8972",
                "43.03100",
                "-87.98000",
                "closure",
                "Wisconsin",
                "I-94 EB",
                "eastbound",
                "Ramp Full Closure on I-94 EB at OFF RAMP TO HAWLEY RD.",
                "2025-11-05T03:00:00Z",
                "2027-11-15T11:30:00Z",
                "Wisconsin DOT",
            ],
        )


if __name__ == "__main__":
    unittest.main()
