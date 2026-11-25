"""Tests for make_unrest.py. Run with: python3 -m unittest discover tools"""

import io
import os
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import make_unrest as unrest  # noqa: E402


def row(
    url,
    place="Lyon, Rhone-Alpes, France",
    feature="-1456928",
    lat="45.75",
    lon="4.85",
    base="141",
    root="14",
    geo="4",
    day="20261125",
    added="20261125170000",
):
    """One line of a GDELT event file, with only the columns the job reads filled in."""
    columns = [""] * 61
    columns[1] = day
    columns[26] = base
    columns[27] = base
    columns[28] = root
    columns[51] = geo
    columns[52] = place
    columns[53] = "FR"
    columns[56] = lat
    columns[57] = lon
    columns[58] = feature
    columns[59] = added
    columns[60] = url
    return "\t".join(columns)


def zipped(text):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr("events.export.CSV", text)
    return buffer.getvalue()


class Sifting(unittest.TestCase):
    def test_a_protest_placed_in_a_town_is_kept_with_its_kind_and_outlet(self):
        found = unrest.sift(row("https://www.lemonde.fr/societe/article/manifestation-lyon.html"))
        self.assertEqual(len(found), 1)
        seen = found[0]
        self.assertEqual(seen.day, "2026-11-25")
        self.assertEqual(seen.kind, "protest")
        self.assertEqual(seen.place, "Lyon, Rhone-Alpes, France")
        self.assertEqual((seen.lat, seen.lon), (45.75, 4.85))
        self.assertEqual(seen.outlet, "lemonde.fr")
        self.assertEqual(seen.key, "-1456928")

    def test_the_kind_is_read_from_the_words_of_the_articles_address(self):
        kinds = {
            "https://a.example/news/thousands-march-through-perth-cbd": "protest",
            "https://a.example/2026/11/25/farmers-rally-outside-parliament.html": "protest",
            "https://a.example/teachers-national-strike-601746": "strike",
            "https://a.example/students-walk-out-over-fees": "strike",
            "https://a.example/greve-generale-a-marseille": "strike",
            "https://a.example/police-fire-tear-gas-at-bishops/": "violent",
            "https://a.example/clashes-erupt-as-protest-turns-violent": "violent",
            "https://a.example/riot-in-city-centre": "violent",
        }
        for url, kind in kinds.items():
            self.assertEqual([seen.kind for seen in unrest.sift(row(url))], [kind], url)

    def test_an_address_that_names_no_unrest_is_dropped(self):
        # GDELT codes all of these as protests. None is one.
        silent = [
            "https://a.example/au-fournils-recipe-for-success-is-built-on-family",
            "https://a.example/news/279452782/washington-pushes-nato-30",
            "https://a.example/story?id=12767157",
            "https://a.example/israeli-air-strike-kills-security-chief",
            "https://a.example/ethiopia-drone-strikes-civilians",
            "https://a.example/journalist-begins-hunger-strike",
            "https://a.example/crowdstrikes-deal-revives-questions",
            "https://a.example/2026/march/council-approves-budget",
            "https://a.example/court-rejects-protest-petition",
            "https://a.example/ghana-protests-to-iraq-over-assault",
            "https://a.example/medvedev-spells-out-when-russia-would-strike",
            # A page listing every article filed under a word is not an article.
            "https://a.example/tag/protests",
        ]
        for url in silent:
            self.assertEqual(unrest.sift(row(url)), [], url)

    def test_everything_else_is_dropped(self):
        dropped = [
            row("https://a.example/protest-in-lyon", root="19"),  # not coded as a protest
            row("https://a.example/protest-in-lyon", geo="1"),  # placed only at a country
            row("https://a.example/protest-in-lyon", geo="2"),  # or a U.S. state
            row("https://a.example/protest-in-lyon", geo="5"),  # or a region elsewhere
            row("https://a.example/protest-in-lyon", lat="", lon=""),  # nowhere at all
            row("", place="Lyon"),  # no article to point to
            row("https://a.example/protest-in-lyon", day="20230627"),  # an old event in today's news
            "too\tfew\tcolumns",
        ]
        self.assertEqual(unrest.sift("\n".join(dropped)), [])

    def test_yesterdays_event_in_todays_news_is_kept(self):
        found = unrest.sift(row("https://a.example/protest-in-lyon", day="20261124"))
        self.assertEqual([seen.day for seen in found], ["2026-11-24"])

    def test_a_place_without_an_identifier_is_keyed_by_where_it_is(self):
        found = unrest.sift(row("https://a.example/protest-in-lyon", feature=""))
        self.assertEqual(found[0].key, "45.75,4.85")


class Reports(unittest.TestCase):
    def seen(self, lines):
        return unrest.sift("\n".join(lines))

    def test_articles_about_one_town_on_one_day_make_one_report(self):
        reports = unrest.reports(
            self.seen(
                [
                    row("https://www.lemonde.fr/manifestation-a-lyon-a"),
                    row("https://www.lemonde.fr/manifestation-a-lyon-a"),  # the same article, twice
                    row("https://www.lefigaro.fr/manifestation-a-lyon-b"),
                    row("https://www.lemonde.fr/manifestation-a-lyon-c"),  # one outlet's second
                    row("https://a.example/protest-in-lyon-d", place="Paris, France", feature="-1456929"),
                ]
            )
        )
        self.assertEqual(len(reports), 2)
        lyon = reports[0]
        self.assertEqual(lyon.place, "Lyon, Rhone-Alpes, France")
        # Two outlets carried it, however many articles they ran.
        self.assertEqual(lyon.outlets, 2)
        self.assertEqual(lyon.id, "2026-11-25/-1456928")
        self.assertEqual(
            lyon.urls,
            ["https://www.lemonde.fr/manifestation-a-lyon-a", "https://www.lefigaro.fr/manifestation-a-lyon-b"],
        )
        self.assertEqual(reports[1].outlets, 1)

    def test_the_same_town_on_another_day_is_another_report(self):
        reports = unrest.reports(
            self.seen(
                [row("https://a.example/protest-in-lyon"), row("https://b.example/protest-in-lyon", day="20261124")]
            )
        )
        self.assertEqual([report.day for report in reports], ["2026-11-25", "2026-11-24"])

    def test_one_outlet_calling_it_violent_does_not_make_it_so(self):
        mixed = self.seen(
            [
                row("https://a.example/riot-in-lyon"),
                row("https://b.example/protest-in-lyon"),
                row("https://c.example/protest-in-lyon"),
            ]
        )
        self.assertEqual(unrest.reports(mixed)[0].kind, "protest")
        agreed = self.seen(
            [
                row("https://a.example/riot-in-lyon"),
                row("https://b.example/clashes-in-lyon"),
                row("https://c.example/protest-in-lyon"),
            ]
        )
        self.assertEqual(unrest.reports(agreed)[0].kind, "violent")
        alone = self.seen([row("https://a.example/riot-in-lyon")])
        self.assertEqual(unrest.reports(alone)[0].kind, "violent")

    def test_a_strike_is_one_when_most_outlets_say_so(self):
        strike = self.seen(
            [
                row("https://a.example/strike-in-lyon"),
                row("https://b.example/strike-in-lyon"),
                row("https://c.example/protest-in-lyon"),
            ]
        )
        self.assertEqual(unrest.reports(strike)[0].kind, "strike")

    def test_no_more_than_five_articles_are_listed(self):
        many = self.seen([row(f"https://outlet{n}.example/protest-in-lyon") for n in range(9)])
        report = unrest.reports(many)[0]
        self.assertEqual(report.outlets, 9)
        self.assertEqual(len(report.urls), 5)


class Slots(unittest.TestCase):
    def test_the_slots_after_one_run_every_quarter_of_an_hour(self):
        self.assertEqual(
            unrest.slots_after("20261125234500", "20261126003000"),
            ["20261126000000", "20261126001500", "20261126003000"],
        )
        self.assertEqual(unrest.slots_after("20261126003000", "20261126003000"), [])

    def test_a_first_run_reaches_back_three_days_and_no_run_more_than_a_week(self):
        first = unrest.slots_to_fetch(None, "20261125190000")
        self.assertEqual(first[0], "20261122191500")
        self.assertEqual(len(first), 3 * 96)
        long_ago = unrest.slots_to_fetch("20261001000000", "20261125190000")
        self.assertEqual(len(long_ago), 7 * 96)
        self.assertEqual(long_ago[-1], "20261125190000")


class Files(unittest.TestCase):
    def test_what_was_seen_is_remembered_between_runs_and_old_days_forgotten(self):
        seen = unrest.sift(
            "\n".join(
                [
                    row("https://a.example/protest-in-lyon"),
                    row("https://b.example/protest-in-lyon", day="20261117", added="20261117120000"),
                ]
            )
        )
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "unrest_seen.tsv")
            unrest.write_seen(path, "20261125190000", seen)
            last, again = unrest.read_seen(path)
            self.assertEqual(last, "20261125190000")
            self.assertEqual(again, seen)
            self.assertEqual(unrest.read_seen(os.path.join(folder, "missing.tsv")), (None, []))
        # Eight days back is past what is kept.
        kept = unrest.forget_old(seen, "20261125190000")
        self.assertEqual([one.day for one in kept], ["2026-11-25"])

    def test_the_file_the_app_reads(self):
        reports = unrest.reports(
            unrest.sift(
                "\n".join(
                    [row("https://www.lemonde.fr/manifestation-a-lyon"), row("https://b.example/protest-in-lyon")]
                )
            )
        )
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "unrest.tsv")
            unrest.write_reports(path, "2026-11-25T19:05:00Z", reports)
            with open(path, encoding="utf-8") as file:
                lines = file.read().splitlines()
        self.assertEqual(lines[0], "#orion-unrest\t1\t2026-11-25T19:05:00Z\t1")
        self.assertEqual(
            lines[1].split("\t"),
            [
                "2026-11-25/-1456928",
                "2026-11-25",
                "45.7500",
                "4.8500",
                "protest",
                "Lyon, Rhone-Alpes, France",
                "2",
                "https://www.lemonde.fr/manifestation-a-lyon https://b.example/protest-in-lyon",
            ],
        )

    def test_each_quarter_hour_has_a_file_in_english_and_one_translated(self):
        self.assertEqual(
            unrest.file_names("20261125190000"),
            ["20261125190000.export.CSV.zip", "20261125190000.translation.export.CSV.zip"],
        )

    def test_a_slot_that_is_missing_or_broken_is_passed_over(self):
        self.assertEqual(unrest.events_text(b"<html>Not found</html>"), "")
        self.assertIn("Lyon", unrest.events_text(zipped(row("https://a.example/protest-in-lyon"))))


if __name__ == "__main__":
    unittest.main()
