import { describe, expect, test } from "bun:test"
import { activeTransfers, formatBytes, preview, relativeTime } from "./model"

describe("Conduit Nova models", () => {
  test("pairing state exposes only pairing status, not device identity", () => {
    expect(preview.pairing.paired).toBe(true)
    expect("deviceId" in preview.pairing).toBe(false)
  })
  test("completed history never becomes a global active-transfer footer", () => {
    expect(activeTransfers(preview)).toHaveLength(0)
    const fixture = { ...preview, transfers: [
      ...preview.transfers,
      { id: "aabbcc03", name: "ongoing.mp3", state: "transferring", terminal: false,
        direction: "outbound" as const, bytes: 10, total: 100, percentage: 10, detail: "" },
    ] }
    expect(activeTransfers(fixture).map((transfer) => transfer.name)).toEqual(["ongoing.mp3"])
  })
  test("bytes format covers zero, MB and GB without crashing", () => {
    expect(formatBytes(0)).toBe("0 B")
    expect(formatBytes(1048576)).toBe("1.00 MB")
    expect(formatBytes(1073741824)).toBe("1.00 GB")
    expect(relativeTime(Date.now())).toBe("Just now")
  })
})
