"""Offline checks for the short front door, documentation routes and brand assets."""
from pathlib import Path
import re
import unittest
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
PAGES = ["README.md", "docs/README.md", "docs/getting-started.md",
         "docs/compatibility.md", "docs/performance.md", "docs/containers.md",
         "docs/production-readiness.md"]
SVG = "{http://www.w3.org/2000/svg}"


def heading_ids(text):
    result = set()
    for heading in re.findall(r"^#{1,6}\s+(.+)$", text, re.M):
        heading = re.sub(r"<[^>]+>", "", heading).lower()
        result.add(re.sub(r"[^\w\- ]", "", heading).replace(" ", "-"))
    return result


class OnboardingTests(unittest.TestCase):
    def test_readme_is_a_short_honest_starting_point(self):
        text = (ROOT / "README.md").read_text()
        self.assertLessEqual(len(text.split()), 450)
        for route in ("docs/getting-started.md", "docs/compatibility.md",
                      "docs/performance.md", "docs/README.md", "CONTRIBUTING.md"):
            self.assertIn(route, text)
        self.assertIn("early-stage", text.lower())
        self.assertIn("-p 127.0.0.1:6379:6379", text)
        self.assertIn("ghcr.io/macel94/rillcache:latest", text)
        self.assertIn('alt="Rillcache"', text)

    def test_moved_documentation_links_and_local_anchors_resolve(self):
        for relative in PAGES:
            page = ROOT / relative
            with self.subTest(page=relative):
                text = page.read_text()
                targets = re.findall(r"\[[^\]]*\]\(([^)]+)\)", text)
                targets += re.findall(r'\bsrc="([^"]+)"', text)
                for target in targets:
                    url = urlsplit(target)
                    if url.scheme or url.netloc:
                        continue
                    resolved = (page.parent / unquote(url.path)).resolve() if url.path else page
                    self.assertTrue(resolved.is_relative_to(ROOT), target)
                    self.assertTrue(resolved.exists(), f"{relative}: {target}")
                    if url.fragment and resolved.suffix == ".md":
                        self.assertIn(unquote(url.fragment), heading_ids(resolved.read_text()), target)

    def test_logos_are_accessible_self_contained_svg(self):
        allowed = {"svg", "title", "desc", "defs", "linearGradient", "stop", "rect", "path", "g", "text"}
        for name in ("rillcache-logo.svg", "rillcache-mark.svg"):
            path = ROOT / "docs/assets" / name
            with self.subTest(asset=name):
                self.assertLess(path.stat().st_size, 12_000)
                root = ET.parse(path).getroot()
                self.assertEqual(root.tag, SVG + "svg")
                self.assertEqual(root.get("role"), "img")
                self.assertTrue(root.find(SVG + "title").text)
                self.assertTrue(root.find(SVG + "desc").text)
                box = list(map(float, root.attrib["viewBox"].split()))
                self.assertEqual(len(box), 4)
                self.assertGreater(box[2], 0)
                self.assertGreater(box[3], 0)
                ids = {node.get("id") for node in root.iter() if node.get("id")}
                self.assertTrue(set(root.attrib["aria-labelledby"].split()) <= ids)
                for node in root.iter():
                    self.assertIn(node.tag.removeprefix(SVG), allowed)
                    for key, value in node.attrib.items():
                        self.assertFalse(key.lower().startswith("on"), key)
                        self.assertNotIn("href", key.lower())
                        self.assertNotIn("http:", value)
                        self.assertNotIn("https:", value)
                        if "url(" in value:
                            self.assertRegex(value, r"^url\(#[\w-]+\)$")
                            self.assertIn(value[5:-1], ids)

    def test_wordmark_contrasts_with_light_and_dark_readme_backgrounds(self):
        def luminance(color):
            rgb = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
            linear = [c / 12.92 if c <= .04045 else ((c + .055) / 1.055) ** 2.4 for c in rgb]
            return sum(c * w for c, w in zip(linear, (.2126, .7152, .0722)))
        root = ET.parse(ROOT / "docs/assets/rillcache-logo.svg").getroot()
        mark = next(node for node in root.iter() if node.get("id") == "wordmark")
        color = luminance(mark.attrib["fill"])
        for background in ("#ffffff", "#0d1117"):
            bg = luminance(background)
            self.assertGreaterEqual((max(color, bg) + .05) / (min(color, bg) + .05), 3)


if __name__ == "__main__":
    unittest.main()
