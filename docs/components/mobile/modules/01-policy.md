# 1. `policy` — the decision core — **Done**

Part of the [Mobile plan](../plan.md).

```
app/src/main/kotlin/com/holyblocker/mobile/policy/
```

`ScanGate` is the load-bearing piece. `AccessibilityService` fires window-content and scroll
events far faster than a human scrolls — several per frame while a list moves — and each one
would otherwise mean a full normalize + lexicon pass on the UI-event path. The gate drops:

| Skip reason | Rule |
|---|---|
| `SELF_PACKAGE` | never scan our own overlay — its text would re-trigger the cover |
| `NO_TEXT` | the node tree yielded nothing usable |
| `DUPLICATE` | identical text within the same app — the verdict cannot differ |
| `DEBOUNCED` | under 300 ms since the last scan *of the same app* |

An app switch bypasses both dedupe and debounce: the whole screen just changed, and that is
the highest-signal moment there is. Only a real evaluation advances the debounce clock, so a
stream of duplicates cannot hold the window open and starve a genuine change.

`BLUR` maps to `COVER` because the MVP overlay is opaque and has no partial-obscure mode;
erring toward covering matches the formation model's "tune blocking for recall".
