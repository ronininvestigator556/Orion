"""Tests for make_advisories.py. Run with: python3 -m unittest discover tools"""

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import make_advisories as advisories  # noqa: E402

COUNTRIES = advisories.read_countries(
    "# a heading\n"
    "DE\tGM\tGermany\t50.962\t9.678\n"
    "FR\tFR\tFrance\t46.696\t2.552\n"
    "CI\tIV\tIvory Coast\t7.500\t-5.500\n"
    "NO\t\tNorway\t61.357\t9.680\n"
    "US\tUS\tUnited States of America\t39.538\t-97.483\n"
)

US = json.dumps(
    [
        {
            "Title": "Germany - Level 2: Exercise Increased Caution",
            "Link": "https://travel.state.gov/destination.deu.html",
            "Category": ["GM"],
            "Summary": "Exercise increased caution<p>in <b>Germany</b> due to&nbsp;<b>terrorism</b>.</p>\n"
            "<p>Terrorist groups keep planning attacks.</p>",
            "Updated": "2026-05-01T20:00:00-04:00",
        },
        {
            "Title": "Norway - Level 1: Exercise Normal Precautions",
            "Link": "https://travel.state.gov/destination.nor.html",
            "Category": ["NO"],
            "Summary": "Exercise normal precautions.",
            "Updated": "2026-03-02T19:00:00-05:00",
        },
        {
            "Title": "Atlantis - Level 4: Do Not Travel",
            "Link": "https://travel.state.gov/destination.atl.html",
            "Category": ["ZZ"],
            "Summary": "Do not travel.",
            "Updated": "2026-03-02T19:00:00-05:00",
        },
    ]
)

UK = json.dumps(
    {
        "links": {
            "children": [
                {
                    "details": {
                        "change_description": "Updated information about demonstrations.",
                        "country": {"name": "France", "slug": "france"},
                    },
                    "public_updated_at": "2026-10-22T16:57:33Z",
                    "web_url": "https://www.gov.uk/foreign-travel-advice/france",
                },
                {
                    "details": {
                        "change_description": "Latest update: entry rules.",
                        "country": {"name": "Côte d'Ivoire", "slug": "cote-d-ivoire"},
                    },
                    "public_updated_at": "2026-09-01T10:00:00Z",
                    "web_url": "https://www.gov.uk/foreign-travel-advice/cote-d-ivoire",
                },
            ]
        }
    }
)

CANADA = json.dumps(
    {
        "data": {
            "FR": {
                "country-iso": "FR",
                "advisory-state": 1,
                "has-regional-advisory": 0,
                "date-published": {"date": "2026-11-25 12:03:46"},
                "eng": {"url-slug": "france", "advisory-text": "Exercise a high degree of caution"},
            },
            "DE": {
                "country-iso": "DE",
                "advisory-state": 0,
                "has-regional-advisory": 1,
                "date-published": {"date": "2026-11-20 09:00:00"},
                "eng": {"url-slug": "germany", "advisory-text": "Take normal security precautions"},
            },
        }
    }
)


class Reading(unittest.TestCase):
    def test_the_united_states_files_advice_under_its_own_codes_and_levels(self):
        found, unplaced = advisories.from_united_states(US, COUNTRIES)
        germany = found["DE"]
        self.assertEqual(germany.level, 2)
        self.assertEqual(germany.advice, "Exercise Increased Caution")
        self.assertEqual(germany.updated, "2026-05-01")
        self.assertEqual(germany.url, "https://travel.state.gov/destination.deu.html")
        # The summary is plain text, without its markup.
        self.assertEqual(
            germany.summary,
            "Exercise increased caution in Germany due to terrorism. "
            "Terrorist groups keep planning attacks.",
        )
        # Norway has no code of its own in the table, so it is found by name.
        self.assertEqual(found["NO"].level, 1)
        # A place the table does not know is reported, not guessed at.
        self.assertEqual(unplaced, ["Atlantis"])

    def test_the_united_kingdom_says_what_last_changed(self):
        found, unplaced = advisories.from_united_kingdom(UK, COUNTRIES)
        self.assertEqual(found["FR"].advice, "Updated information about demonstrations.")
        self.assertEqual(found["FR"].updated, "2026-10-22")
        self.assertEqual(found["FR"].url, "https://www.gov.uk/foreign-travel-advice/france")
        # A country it calls by another name is still found.
        self.assertIn("CI", found)
        self.assertEqual(unplaced, [])

    def test_canada_gives_a_level_and_says_when_regions_differ(self):
        found, _ = advisories.from_canada(CANADA, COUNTRIES)
        self.assertEqual(found["FR"].advice, "Exercise a high degree of caution")
        self.assertEqual(found["FR"].updated, "2026-11-25")
        self.assertEqual(found["FR"].url, "https://travel.gc.ca/destinations/france")
        self.assertEqual(
            found["DE"].advice, "Take normal security precautions (some regions differ)"
        )

    def test_names_are_matched_whatever_their_accents_and_small_words(self):
        same = ["Côte d'Ivoire", "Cote d Ivoire", "Ivory Coast", "The Ivory Coast"]
        self.assertEqual({advisories.known_name(name) for name in same}, {"ivory coast"})
        self.assertEqual(
            advisories.known_name("Saint Kitts & Nevis"), advisories.known_name("St Kitts and Nevis")
        )
        self.assertEqual(advisories.known_name("The Gambia"), advisories.known_name("Gambia"))

    def test_markup_is_taken_out_and_long_text_cut_at_a_sentence(self):
        self.assertEqual(advisories.plain_text("<p>One&nbsp;two.</p><p>Three &amp; four.</p>"), "One two. Three & four.")
        long = "First sentence here. " + "word " * 200 + "end."
        cut = advisories.plain_text(long, limit=60)
        self.assertEqual(cut, "First sentence here.")


class Joining(unittest.TestCase):
    def test_each_country_gets_one_line_with_what_every_government_says(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "advisories.tsv")
            count = advisories.write(
                path,
                "2026-11-25T20:00:00Z",
                COUNTRIES,
                advisories.from_united_states(US, COUNTRIES)[0],
                advisories.from_united_kingdom(UK, COUNTRIES)[0],
                advisories.from_canada(CANADA, COUNTRIES)[0],
            )
            with open(path, encoding="utf-8") as file:
                lines = file.read().splitlines()
        # The United States has advice from nobody here, so it has no line.
        self.assertEqual(count, 4)
        self.assertEqual(lines[0], "#orion-advisories\t1\t2026-11-25T20:00:00Z\t4")
        germany = [line.split("\t") for line in lines if line.startswith("DE\t")][0]
        self.assertEqual(
            germany,
            [
                "DE",
                "Germany",
                "50.962",
                "9.678",
                # United States: level, advice, when, where to read it, summary.
                "2",
                "Exercise Increased Caution",
                "2026-05-01",
                "https://travel.state.gov/destination.deu.html",
                "Exercise increased caution in Germany due to terrorism. "
                "Terrorist groups keep planning attacks.",
                # United Kingdom: nothing.
                "",
                "",
                "",
                # Canada.
                "Take normal security precautions (some regions differ)",
                "2026-11-20",
                "https://travel.gc.ca/destinations/germany",
            ],
        )
        france = [line.split("\t") for line in lines if line.startswith("FR\t")][0]
        # No United States advice in this test, so no level.
        self.assertEqual(france[4], "")
        self.assertEqual(france[9], "Updated information about demonstrations.")


if __name__ == "__main__":
    unittest.main()
