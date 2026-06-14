# Flow: Partner Setup

**Trigger:** user clicks "Add accountability partner" during onboarding, or from the
Accountability section of the Settings panel.

See [decisions/accountability.md](../../decisions/accountability.md) for the reasoning
behind each step.

---

## User side

### Step 1 — Generate invite link

```
User clicks "Add accountability partner"
  → app generates a unique one-time invite token (UUID, stored locally)
  → invite link constructed: https://holyblocker.app/invite/{token}
  → screen shows:
      - the link (copyable)
      - "Share this with someone you trust"
      - three quick-send options: copy link / open email / open messages
      - a plain-English sentence: "They don't need to install anything."
  → user sends via whatever channel they use with that person
```

No partner email is entered at this step. The user does not need to know the partner's
email address in advance — they send the link however they communicate naturally.

### Step 2 — Waiting state

```
After sharing:
  → settings panel shows "Invite pending — waiting for [no name yet] to confirm"
  → invite expires after 7 days if not accepted
  → user can cancel the invite or generate a new one at any time
  → if the invite expires: notification shown, user can generate a new one
```

### Step 3 — Partner confirms (see partner side below)

```
Partner opens link and confirms
  → app receives confirmation over polling or push (implementation TBD)
  → settings panel updates to "Partner: [Partner's name]  ✓ Active"
  → streak counter starts from 0
  → user receives an in-app notification: "[Name] accepted — accountability is active"
```

---

## Partner side (web, no app install required)

### Step 1 — Landing page

Partner opens the invite link in any browser on any device.

```
Page shows:
  - "[User's display name] has invited you to be their accountability partner."
  - Three plain bullet points:
      • "You'll receive a weekly summary showing how many times content was blocked."
      • "You'll be notified if [Name] tries to disable their protection."
      • "You won't see any URLs, content, or details — only counts."
  - "You can opt out any time with one click."
  - [ Confirm ] button
```

The page does not require login, account creation, or app install. It requires only
that the partner enter their name (for display in the user's app) and an email address
(for the weekly summary and gate notifications).

### Step 2 — Confirmation

```
Partner enters name + email
  → confirmation email sent to partner's address
  → partner clicks confirm in that email
  → server marks invite token as accepted, stores partner contact
  → user's app notified
```

Two-step email confirmation (enter address → click link in email) prevents the user
from entering a wrong address silently. The partner must take an action in their own
inbox.

---

## Weekly summary delivery (ongoing)

```
Every Monday, 09:00 local time (user's timezone):
  → summary email sent to partner:

      Subject: Holy Blocker — weekly update for [User's name]

      This week:
        Blocked:           14
        Warned:             3
        Override attempts:  0
        Current streak:    12 days

      [Send encouragement →]   [Unsubscribe]
```

The "Send encouragement" link opens a lightweight web page where the partner can send
a short message. The message is delivered as an in-app notification to the user.

---

## Gate attempt notification (on trigger)

```
User opens voice gate (attempts to switch to Off mode)
  → immediately: notification email sent to partner:

      Subject: Holy Blocker — protection override attempt

      [User's name] attempted to disable Holy Blocker protection.
      Time: [timestamp]
      Outcome: [Cancelled by user / Protection disabled]

      Current streak: [n] days  ← resets to 0 if outcome is "disabled"

      [Send encouragement →]   [Unsubscribe]
```

Sent immediately, not batched. The partner should know about a gate attempt while
the context is still fresh enough for a meaningful conversation.

---

## Streak

```
Streak increments:
  → +1 day at midnight (user's timezone) if no successful gate clear occurred that day

Streak resets to 0:
  → when protection mode transitions to Off (gate cleared successfully)

Streak does NOT reset on:
  → warn events (scanner doing its job)
  → cancelled gate attempts (friction working as intended)
  → switching between Full and Warn modes

Milestone notifications sent to both user and partner:
  → 7 days, 30 days, 90 days, 180 days, 1 year
  → in-app notification for user
  → email for partner (same format as encouragement, no action required)
```

---

## Ending the partnership

### Partner opts out

```
Partner clicks unsubscribe in any email
  → immediately removed from all future notifications
  → user receives in-app notification:
      "[Name] has stepped down as your accountability partner.
       You're in solo mode. You can add a new partner any time."
  → streak is preserved (the covenant continues; only the partner changed)
  → invite token invalidated
```

### User removes partner

```
User clicks "Remove partner" in settings
  → confirmation prompt shown
  → on confirm: partner receives a single notification email:
      "[User's name] has ended the accountability partnership.
       You will receive no further notifications."
  → user returns to solo mode
  → streak preserved
```

---

## Related

- [decisions/accountability.md](../../decisions/accountability.md) — why the model is
  designed this way, what partners do and don't receive
- [flows/override-gate.md](override-gate.md) — the voice gate flow that triggers
  partner notifications
- [desktop/PLAN.md](../../components/desktop/plan.md) — implementation steps for the settings panel
  and IPC
