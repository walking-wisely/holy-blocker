# Foundation

## Why this project exists

> "I made a covenant with my eyes not to look lustfully at a young woman."
> — Job 31:1 (NIV)

*(All scripture quotations in this document are from the New International Version.
For licensing notes on embedding NIV text in the distributed application, see
[decisions/verse-pools.md](decisions/verse-pools.md).)*

Job's covenant was not a rule imposed from outside. It was a personal commitment, made
before God, about what he would allow his eyes and his mind to dwell on. That covenant
required nothing more than his own will — because in Job's world, the content was not
delivered directly to a screen in front of him around the clock.

In ours, it is.

Holy Blocker exists because making that same covenant on a modern desktop or phone is
much harder than deciding to make it — and every tool available to help was either
locked behind a paid subscription, too shallow to block visual content (images and
video, not just domain names), or both.

The project is built by someone who wanted to keep that covenant and couldn't find a
tool that was adequate and free. It is built for others in the same position.

---

## What "covenant with my eyes" actually requires technically

Most content blockers operate only at the domain level. They can prevent the browser
from loading `example.com`, but they cannot see what the page looks like, cannot block
images served from a CDN with a clean-looking domain, and cannot detect content that
was downloaded locally before the blocker was installed.

Scripture does not restrict the covenant to "domains you navigate to in a browser." The
eye does not distinguish a network request from a file on disk. The blocker should not
either. This is why Holy Blocker intercepts at multiple layers:

- **Network level** — traffic is intercepted before it reaches the browser, including
  inside HTTPS streams. Known-unholy domains are dropped at the packet level. Unknown
  traffic is decrypted, scanned for text and images, and blocked or flagged before
  the browser renders it.
- **Screen-capture level** — a daemon watches what is actually on screen, regardless
  of how it arrived. This catches cached pages, locally downloaded content, native
  apps, and anything else that bypasses the network path.

Neither layer alone is sufficient. Together they approximate what a vigilant eye-gate
actually needs.

> "If your right eye causes you to stumble, gouge it out and throw it away. It is
> better for you to lose one part of your body than for your whole body to be thrown
> into hell."
> — Matthew 5:29 (NIV)

Jesus' point was not that self-mutilation is the answer. It was that the cost of
removing the source of temptation is always less than the cost of giving in to it. A
tool that almost blocks is not the same as one that blocks.

---

## What we block — and why the scope is broader than pornography

The obvious target is sexually explicit content. That is where the most acute harm is,
and it is the first category the project addresses.

But the project's scope is broader. The criterion is not "legally obscene" or
"clinically pornographic." The criterion is:

> "Finally, brothers and sisters, whatever is true, whatever is noble, whatever is
> right, whatever is pure, whatever is lovely, whatever is admirable — if anything is
> excellent or praiseworthy — think about such things."
> — Philippians 4:8 (NIV)

Content that does not meet that standard — degrading material, gratuitous violence,
content designed to enflame lust even without explicit imagery — is in scope. Building
classifiers broad enough to cover it reliably takes time and a training dataset that
does not yet exist. The project will reach that wider scope incrementally. But the
design principles and theological rationale are fixed from the start: the blocker
should guard the mind against what is *unholy*, not just what is *illegal*.

> "Above all else, guard your heart, for everything you do flows from it."
> — Proverbs 4:23 (NIV)

---

## Why accountability is built into the design

Willpower alone is not the model this project assumes. Scripture does not assume it
either.

> "As iron sharpens iron, so one person sharpens another."
> — Proverbs 27:17 (NIV)

> "Therefore confess your sins to each other and pray for each other so that you may
> be healed."
> — James 5:16 (NIV)

This is why the system notifies a designated partner when the user attempts to disable
protection — not only if they succeed, but on the attempt itself. The attempt is
meaningful. Silence on a failed bypass is a hidden near-miss, and hidden near-misses
erode accountability.

The voice gate (speaking a scripture passage aloud to disable the system) is not
primarily a security measure. It is a deliberate pause — a moment of engagement with
God's word at exactly the moment when the temptation to lower the guard is highest.
The friction is the feature.

See [decisions/accountability.md](decisions/accountability.md) for the design of the
accountability model, and [decisions/protection-modes.md](decisions/protection-modes.md)
for why the voice gate fires only on the transition to `off`.

---

## What Holy Blocker does not claim to prevent — and where it tries anyway

An admin or owner of the device can always uninstall any application. There is no
technical countermeasure that fully prevents this. Holy Blocker does not pretend
otherwise.

What it does is make uninstall *costly* for the people who are most likely to act
impulsively. On each supported platform the project will use the strongest mechanisms
available:

- **Windows** — hiding from the standard Add/Remove Programs list, requiring
  administrator elevation to uninstall, and (where achievable without a kernel driver)
  protecting the installation directory from deletion by the user's own account.
- **Android** — requesting Device Administrator privileges so uninstall requires an
  explicit revocation step before the standard uninstall flow proceeds.
- **Future platforms** — the same principle applies: maximum friction available within
  the platform's permission model, short of techniques that would require a kernel
  driver or MDM enrollment.

These measures will not stop a technically capable and determined person. They will
stop an impulsive decision made at 2 am. That is the realistic target.

The right frame is not "unbreakable lock" but "costly enough to be deliberate." The
voice gate, the accountability notification, and the platform-level install protection
work together to insert a deliberate pause at every exit point. The number of steps
required to disable or remove the tool is the measure of success, not the theoretical
impossibility of doing so.

---

## Local-first and private

> "But when you pray, go into your room, close the door and pray to your Father, who
> is unseen."
> — Matthew 6:6 (NIV)

A person's struggle with sexual sin is between them and God first, and secondarily
between them and a trusted accountability partner. It is not data to be processed by a
cloud service.

Holy Blocker makes all blocking decisions on-device. No screenshots, no OCR text, no
browsing content, and no block events are sent to a server. The accountability
notification goes to the partner the user explicitly designates — it is not logged by
the project, not aggregated, not analyzed. The verse shown during a block is embedded
in the app bundle; no network call is made to select or retrieve it.

This is not just a privacy posture. It is an extension of the same respect for the
privacy of conscience that applies to prayer.

---

## Related documents

- [Architecture](architecture.md) — how the technical layers implement the two-path
  blocking model described above
- [Network Pipeline](network-pipeline.md) — the five phases of network interception
- [decisions/verse-selection.md](decisions/verse-selection.md) — why verses are shown
  during warn events and the voice gate, and how they are selected
- [decisions/protection-modes.md](decisions/protection-modes.md) — the three operating
  modes and why the gate only fires on the transition to `off`
- [decisions/accountability.md](decisions/accountability.md) — partner notifications
  and the accountability model
