# GitHub Pages site

Static HTML, CSS, and JavaScript; no build step or runtime dependencies.
The existing Pages workflow publishes this directory when `site/**` changes
land on `main`.

Preview from the repository root:

```sh
python -m http.server 4173 --bind 127.0.0.1 --directory site
```

Open http://localhost:4173. Assets use relative URLs so the page also works
under the `/muckdb/` GitHub Pages project path.

## Design

The original copy, section order, author’s note, installation instructions,
and complete screenshot gallery are retained. The opening pairs the terminal
with real map and timeline captures in a layered composition. The palette is
petrol green, pale mint, and warm copper. Space Grotesk carries the opening
headline; DM Sans carries the prose. Both fonts are served locally, with their
OFL licenses in `assets/fonts/`.

The screenshots are existing real product captures with demo data. All original
gallery examples remain available, with two additional previews in the opening.
The gallery lightbox supports arrow navigation, Escape, focus trapping, and
focus restoration. The site does not embed a live database or connect to the
visitor's localhost daemon.

The opening animation runs once, finishes in under two seconds, and can be
replayed. It only animates transforms and opacity, settles when the tab is
hidden, and respects changes to the reduced-motion preference. Content stays
visible if JavaScript is disabled.

## Editing and checking

- `index.html`: original content, gallery, and opening composition.
- `style.css`: original layout styles followed by the revised opening and
  palette, responsive layouts, and reduced-motion behavior.
- `site.js`: gallery lightbox and the finite opening animation.
- `assets/screenshots/`: original product captures; retain their provenance.

After editing, check narrow mobile and desktop layouts, every gallery image,
arrow navigation, lightbox Escape/close and focus restoration, image loading,
internal links, and the opening with normal and reduced motion.
