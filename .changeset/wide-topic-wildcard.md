---
"@nomicfoundation/edr": patch
---

Fixed an empty array in a log filter's `topics` matching no logs. It now matches any topic in that position, like `null`
