# Decision: Verse Pools (Curated NIV Text)

This document is the canonical source for the verse pools used by the warn interstitial
and the override gate. The JSON files at `data/verses/warn.json` and
`data/verses/gate.json` are populated from this list.

All verses in this document are from the New International Version (NIV), used here
for documentation and curation reference purposes. See the **Translation and licensing**
section below before embedding these texts in the distributed application.

See [verse-selection.md](verse-selection.md) for the selection logic, category-to-pool
mapping, and storage format.

---

## Translation and licensing

**NIV** — The New International Version is owned by Biblica, Inc. Quoting NIV in
documentation (Markdown files, READMEs, design docs) is fine under Biblica's standard
quotation policy — up to 500 verses, with attribution, no written permission needed.
This document and `foundation.md` are covered by that policy.

The distinction that matters for the codebase: *embedding verse text in a distributed
application binary or bundle* is a separate use case and likely requires a commercial
license from Biblica. Compiling the JSON pools into the Rust binary via `include_str!`
or bundling them into the Electron/Vite build is that use case. Before shipping, verify
the current terms at biblica.com or contact them directly.

**If a license cannot be obtained for the binary:** the **World English Bible (WEB)** is a
contemporary public-domain translation with no usage restrictions. Its phrasing is
close to modern English and readable without archaic language. The verse IDs and
references in this document remain valid; only the `"text"` field would need to be
re-sourced from the WEB. The WEB is a safe fallback for v1.

**KJV** is also public domain but its archaic phrasing ("thee", "thou", "doest") makes
it a worse fit for the interstitial and gate UX where readability under emotional
pressure matters.

### i18n and future translations

As the project grows to support users whose primary language is not English, the verse
pool will need to be internationalised. The planned approach:

- The JSON structure gains a top-level `"translation"` key and a locale code, e.g.
  `"translation": "NIV"`, `"locale": "en"`.
- A separate pool file exists per locale: `data/verses/warn.en.json`,
  `data/verses/warn.ru.json`, etc.
- Each locale's pool references the dominant translation for that language community
  (e.g. Синодальный перевод for Russian, Reina-Valera for Spanish). Licensing must be
  verified per translation.
- The app selects the pool file matching the user's configured locale, falling back to
  `en` if no pool exists for that locale.

This structure does not need to be implemented in v1 — the single `warn.json` /
`gate.json` path is sufficient — but the `"id"` field is designed to be
translation-agnostic so verse IDs are stable across locales.

---

---

## Warn interstitial pool (`data/verses/warn.json`)

Shown when a content scan returns a Warn verdict. Verses are short enough to read in
a few seconds and redirect attention without being harsh or accusatory.

### Category: `ExplicitAct` — purity of mind, fleeing lust

```json
[
  {
    "id": "1co6-18",
    "reference": "1 Corinthians 6:18",
    "text": "Flee from sexual immorality. All other sins a person commits are outside the body, but whoever sins sexually, sins against their own body."
  },
  {
    "id": "mt5-28",
    "reference": "Matthew 5:28",
    "text": "But I tell you that anyone who looks at a woman lustfully has already committed adultery with her in his heart."
  },
  {
    "id": "2ti2-22",
    "reference": "2 Timothy 2:22",
    "text": "Flee the evil desires of youth and pursue righteousness, faith, love and peace, along with those who call on the Lord out of a pure heart."
  },
  {
    "id": "ro13-14",
    "reference": "Romans 13:14",
    "text": "Rather, clothe yourselves with the Lord Jesus Christ, and do not think about how to gratify the desires of the flesh."
  },
  {
    "id": "1th4-3-4",
    "reference": "1 Thessalonians 4:3–4",
    "text": "It is God's will that you should be sanctified: that you should avoid sexual immorality; that each of you should learn to control your own body in a way that is holy and honorable."
  }
]
```

### Category: `Nudity` — the body as a temple, modesty

```json
[
  {
    "id": "1co6-19-20",
    "reference": "1 Corinthians 6:19–20",
    "text": "Do you not know that your bodies are temples of the Holy Spirit, who is in you, whom you have received from God? You are not your own; you were bought at a price. Therefore honor God with your bodies."
  },
  {
    "id": "job31-1",
    "reference": "Job 31:1",
    "text": "I made a covenant with my eyes not to look lustfully at a young woman."
  },
  {
    "id": "ps101-3",
    "reference": "Psalm 101:3",
    "text": "I will not look with approval on anything that is vile. I hate what faithless people do; I will have no part in it."
  },
  {
    "id": "php4-8",
    "reference": "Philippians 4:8",
    "text": "Finally, brothers and sisters, whatever is true, whatever is noble, whatever is right, whatever is pure, whatever is lovely, whatever is admirable — if anything is excellent or praiseworthy — think about such things."
  }
]
```

### Category: `AdultPlatform` / `CommercialAdult` — guarding your eyes, what you dwell on

```json
[
  {
    "id": "mt6-22-23",
    "reference": "Matthew 6:22–23",
    "text": "The eye is the lamp of the body. If your eyes are healthy, your whole body will be full of light. But if your eyes are unhealthy, your whole body will be full of darkness."
  },
  {
    "id": "ps119-37",
    "reference": "Psalm 119:37",
    "text": "Turn my eyes away from worthless things; preserve my life according to your word."
  },
  {
    "id": "pr4-25",
    "reference": "Proverbs 4:25",
    "text": "Let your eyes look straight ahead; fix your gaze directly before you."
  },
  {
    "id": "1jn2-16",
    "reference": "1 John 2:16",
    "text": "For everything in the world — the lust of the flesh, the lust of the eyes, and the pride of life — comes not from the Father but from the world."
  },
  {
    "id": "col3-2",
    "reference": "Colossians 3:2",
    "text": "Set your minds on things above, not on earthly things."
  }
]
```

### Category: `fallback` — guarding the heart and mind generally

Used when no dominant category is identified or the evidence is mixed.

```json
[
  {
    "id": "pr4-23",
    "reference": "Proverbs 4:23",
    "text": "Above all else, guard your heart, for everything you do flows from it."
  },
  {
    "id": "php4-8",
    "reference": "Philippians 4:8",
    "text": "Finally, brothers and sisters, whatever is true, whatever is noble, whatever is right, whatever is pure, whatever is lovely, whatever is admirable — if anything is excellent or praiseworthy — think about such things."
  },
  {
    "id": "ro12-2",
    "reference": "Romans 12:2",
    "text": "Do not conform to the pattern of this world, but be transformed by the renewing of your mind. Then you will be able to test and approve what God's will is — his good, pleasing and perfect will."
  },
  {
    "id": "ps19-14",
    "reference": "Psalm 19:14",
    "text": "May these words of my mouth and this meditation of my heart be pleasing in your sight, Lord, my Rock and my Redeemer."
  },
  {
    "id": "ga5-16",
    "reference": "Galatians 5:16",
    "text": "So I say, walk by the Spirit, and you will not gratify the desires of the flesh."
  }
]
```

---

## Override gate pool (`data/verses/gate.json`)

Shown when the user attempts to disable protection. Longer, more reflective passages —
the user reads aloud as part of the voice gate. These are not warnings; they are
invitations to pause and reconnect with the commitment they made.

Pool size is smaller (3–5 passages) because the reading burden is higher and repetition
in this context is less of a concern — encountering the same passage again at a moment
of temptation is not a problem.

```json
[
  {
    "id": "1co10-13",
    "reference": "1 Corinthians 10:13",
    "text": "No temptation has overtaken you except what is common to mankind. And God is faithful; he will not let you be tempted beyond what you can bear. But when you are tempted, he will also provide a way out so that you can endure it."
  },
  {
    "id": "heb4-15-16",
    "reference": "Hebrews 4:15–16",
    "text": "For we do not have a high priest who is unable to empathize with our weaknesses, but we have one who has been tempted in every way, just as we are — yet he did not sin. Let us then approach God's throne of grace with confidence, so that we may receive mercy and find grace to help us in our time of need."
  },
  {
    "id": "ps46-1",
    "reference": "Psalm 46:1",
    "text": "God is our refuge and strength, an ever-present help in trouble."
  },
  {
    "id": "job31-1-4",
    "reference": "Job 31:1–4",
    "text": "I made a covenant with my eyes not to look lustfully at a young woman. For what is our lot from God above, our heritage from the Almighty on high? Is it not ruin for the wicked, disaster for those who do wrong? Does he not see my ways and count my every step?"
  },
  {
    "id": "1co6-19-20",
    "reference": "1 Corinthians 6:19–20",
    "text": "Do you not know that your bodies are temples of the Holy Spirit, who is in you, whom you have received from God? You are not your own; you were bought at a price. Therefore honor God with your bodies."
  }
]
```

---

## Curation notes

- All text in this document is NIV (for curation reference). See the translation and
  licensing section above for what to use in the distributed binary.
- Do not mix translations within a single locale's pool.
- Pool size target for warn: 4–6 verses per category (enough variety to avoid repetition
  within a session; small enough to maintain curation quality).
- Pool size target for gate: 3–5 passages. Longer passages are preferred here.
- Verse IDs use the format `bookabbrev-chapter-verse` (e.g. `1co6-18`, `pr4-23`).
  Use abbreviated book names from standard concordance abbreviations.
- The same verse may appear in multiple category pools if it is genuinely applicable
  (e.g. Philippians 4:8 appears in both `Nudity` and `fallback`). Avoid gratuitous
  duplication.
- Do not include verses that name specific sins explicitly — the verse text will be
  shown on-screen and should redirect attention, not re-expose content or shame the
  user.
