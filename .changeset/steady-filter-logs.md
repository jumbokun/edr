---
"@nomicfoundation/edr": patch
---

Fixed `eth_getFilterLogs` returning only the logs since the last poll and consuming them, so a second call returned none. It now returns all logs matching the filter
