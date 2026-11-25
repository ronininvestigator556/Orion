"""Tests for make_conflicts.py. Run with: python3 -m unittest discover tools"""

import datetime
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import make_conflicts as conflicts  # noqa: E402

COLUMNS = (
    "id,code_status,type_of_violence,conflict_new_id,conflict_name,side_a,side_b,"
    "source_headline,where_prec,where_coordinates,latitude,longitude,country,"
    "date_start,date_end,best,high,low"
)


def line(
    event_id,
    kind="1",
    conflict="218",
    name="Russia - Ukraine",
    sides=("Government of Russia", "Government of Ukraine"),
    precision="1",
    place="Kharkiv town",
    lat="49.98",
    lon="36.25",
    country="Ukraine",
    day="2026-10-06",
    deaths=("3", "5", "2"),
    status="Clear",
):
    best, high, low = deaths
    return (
        f'{event_id},{status},{kind},{conflict},"{name}",{sides[0]},{sides[1]},'
        f'"A headline naming John Smith",{precision},{place},{lat},{lon},{country},'
        f"{day} 00:00:00.000,{day} 00:00:00.000,{best},{high},{low}"
    )


def file(*lines):
    return "﻿" + COLUMNS + "\n" + "\n".join(lines) + "\n"


class Reading(unittest.TestCase):
    def test_an_incident_says_where_when_between_whom_and_how_many_died(self):
        events = conflicts.read_events(file(line("644785")))
        self.assertEqual(len(events), 1)
        event = events[0]
        self.assertEqual(event.id, "644785")
        self.assertEqual(event.day, "2026-10-06")
        self.assertEqual((event.lat, event.lon), (49.98, 36.25))
        self.assertEqual(event.precision, 1)
        self.assertEqual(event.kind, "state")
        self.assertEqual(event.place, "Kharkiv town")
        self.assertEqual(event.conflict, "Russia - Ukraine")
        self.assertEqual((event.side_a, event.side_b), ("Government of Russia", "Government of Ukraine"))
        self.assertEqual((event.best, event.low, event.high), (3, 2, 5))
        self.assertTrue(event.checked)

    def test_the_headlines_a_record_was_drawn_from_are_never_carried(self):
        events = conflicts.read_events(file(line("1")))
        self.assertNotIn("John Smith", repr(events))

    def test_the_three_kinds_and_records_still_being_checked(self):
        events = conflicts.read_events(
            file(line("1", kind="1"), line("2", kind="2"), line("3", kind="3", status="Check dyad"))
        )
        self.assertEqual([event.kind for event in events], ["state", "groups", "civilians"])
        self.assertEqual([event.checked for event in events], [True, True, False])

    def test_sides_not_yet_identified_are_put_in_words(self):
        events = conflicts.read_events(
            file(
                line("1", kind="2", name="XXX100 - XXX100", sides=("XXX100", "XXX100"), country="Colombia"),
                line("2", kind="3", name="XXX652 - Civilians", sides=("XXX652", "Civilians"), country="Syria"),
            )
        )
        self.assertEqual(events[0].conflict, "Unidentified armed groups in Colombia")
        self.assertEqual(events[0].side_a, "an unidentified armed group")
        self.assertEqual(events[1].conflict, "Unidentified armed group against civilians in Syria")
        self.assertEqual(events[1].side_b, "Civilians")

    def test_records_that_cannot_be_read_are_passed_over(self):
        bad = [line("1", lat="north"), line("2", day="soon"), line("3", kind="9"), line("", kind="1")]
        self.assertEqual(conflicts.read_events(file(*bad)), [])


class Gathering(unittest.TestCase):
    def test_a_conflict_is_one_line_for_each_country_it_is_fought_in(self):
        events = conflicts.read_events(
            file(
                line("1", lat="49.0", lon="36.0", deaths=("3", "5", "2")),
                line("2", lat="50.0", lon="37.0", deaths=("10", "12", "8")),
                line("3", lat="51.0", lon="38.0", deaths=("1", "1", "1")),
                # Placed only to the country: it counts, but does not move the mark.
                line("4", lat="10.0", lon="10.0", precision="6", deaths=("2", "2", "2")),
                line("5", country="Russia", lat="55.0", lon="37.0", deaths=("4", "4", "4")),
                line("6", kind="2", conflict="900", name="A - B", sides=("A", "B"), country="Mali", deaths=("7", "9", "7")),
            )
        )
        found = conflicts.conflicts(events)
        self.assertEqual([(one.name, one.country) for one in found], [
            ("Russia - Ukraine", "Ukraine"),
            ("A - B", "Mali"),
            ("Russia - Ukraine", "Russia"),
        ])
        ukraine = found[0]
        self.assertEqual(ukraine.id, "218/Ukraine")
        self.assertEqual((ukraine.lat, ukraine.lon), (50.0, 37.0))
        self.assertEqual(ukraine.events, 4)
        self.assertEqual((ukraine.best, ukraine.low, ukraine.high), (16, 13, 20))
        self.assertEqual(ukraine.kind, "state")
        self.assertEqual(ukraine.sides, "Government of Russia – Government of Ukraine")

    def test_only_the_commonest_pairs_of_sides_are_listed(self):
        lines = [
            line(str(number), sides=(f"Group {number}", "Civilians"), kind="3")
            for number in range(6)
        ]
        found = conflicts.conflicts(conflicts.read_events(file(*lines)))
        self.assertTrue(found[0].sides.endswith("; and 2 more"), found[0].sides)


class Files(unittest.TestCase):
    def test_the_monthly_files_are_looked_for_newest_first_across_a_new_year(self):
        names = conflicts.file_names(datetime.date(2027, 2, 10))
        self.assertEqual(
            names[:4],
            [
                "GEDEvent_v27_0_1.csv",
                "GEDEvent_v26_0_12.csv",
                "GEDEvent_v26_0_11.csv",
                "GEDEvent_v26_0_10.csv",
            ],
        )

    def test_the_two_files_the_app_reads(self):
        events = conflicts.read_events(
            file(
                line("1", day="2026-10-06"),
                line("2", day="2026-08-20", precision="6"),
            )
        )
        with tempfile.TemporaryDirectory() as folder:
            events_path = os.path.join(folder, "conflict_events.tsv")
            conflicts_path = os.path.join(folder, "armed_conflicts.tsv")
            covered = conflicts.period(["GEDEvent_v26_0_10.csv", "GEDEvent_v26_0_8.csv"], events)
            shown = conflicts.write_events(events_path, "2026-11-25T23:00:00Z", events, covered)
            conflicts.write_conflicts(
                conflicts_path, "2026-11-25T23:00:00Z", conflicts.conflicts(events), covered
            )
            with open(events_path, encoding="utf-8") as opened:
                event_lines = opened.read().splitlines()
            with open(conflicts_path, encoding="utf-8") as opened:
                conflict_lines = opened.read().splitlines()
        # The incident placed only to its country is counted but not marked.
        self.assertEqual(shown, 1)
        self.assertEqual(
            event_lines[0], "#orion-conflict-events\t1\t2026-11-25T23:00:00Z\t1\t2026-08-01\t2026-10-06"
        )
        self.assertEqual(
            event_lines[1].split("\t"),
            [
                "1",
                "2026-10-06",
                "49.98000",
                "36.25000",
                "1",
                "state",
                "Ukraine",
                "Kharkiv town",
                "Russia - Ukraine",
                "Government of Russia",
                "Government of Ukraine",
                "3",
                "2",
                "5",
                "1",
            ],
        )
        self.assertEqual(
            conflict_lines[0], "#orion-armed-conflicts\t1\t2026-11-25T23:00:00Z\t1\t2026-08-01\t2026-10-06"
        )
        self.assertEqual(
            conflict_lines[1].split("\t"),
            [
                "218/Ukraine",
                "49.9800",
                "36.2500",
                "state",
                "Ukraine",
                "Russia - Ukraine",
                "Government of Russia – Government of Ukraine",
                "2",
                "6",
                "4",
                "10",
            ],
        )


if __name__ == "__main__":
    unittest.main()
