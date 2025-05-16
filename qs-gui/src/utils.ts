/// https://stackoverflow.com/questions/10420352/converting-file-size-in-bytes-to-human-readable-string

/**
 * Format bytes as human-readable text.
 *
 * @param bytes Number of bytes.
 * @param si True to use metric (SI) units, aka powers of 1000. False to use
 *           binary (IEC), aka powers of 1024.
 * @param dp Number of decimal places to display.
 *
 * @return Formatted string.
 */
export function humanFileSize(bytes: number, si = false, dp = 1) {
    const thresh = si ? 1000 : 1024

    if (Math.abs(bytes) < thresh) {
        return bytes + " B"
    }

    const units = si
        ? ["kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"]
        : ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB"]
    let u = -1
    const r = 10 ** dp

    do {
        bytes /= thresh
        ++u
    } while (
        Math.round(Math.abs(bytes) * r) / r >= thresh &&
        u < units.length - 1
    )

    return bytes.toFixed(dp) + " " + units[u]
}

/// Get the name of the file or directory given a path.
/// This should work on both Windows and Unix-like systems.
export function getFileNameFromPath(path: string): string {
    path = path.replace(/\\/g, "/")
    let parts = path.split("/")
    return parts[parts.length - 1]
}

/// Get a human readable format for the time remaining.
const UNITS: [number, string, string][] = [
    [365 * 24 * 60 * 60, "year", "y"],
    [7 * 24 * 60 * 60, "week", "w"],
    [24 * 60 * 60, "day", "d"],
    [60 * 60, "hour", "h"],
    [60, "minute", "m"],
    [1, "second", "s"],
]

export function humanDuration(seconds: number, compact = false): string {
    let idx = 0
    for (let i = 0; i < UNITS.length; i++) {
        idx = i
        const [cur] = UNITS[i]
        const next = UNITS[i + 1]
        if (
            next &&
            seconds + Math.floor(next[0] / 2) >= cur + Math.floor(cur / 2)
        ) {
            break
        }
    }

    const [unit, name, alt] = UNITS[idx]
    let t = Math.round(seconds / unit)
    if (idx < UNITS.length - 1) {
        t = Math.max(t, 2)
    }

    if (compact) {
        return `${t}${alt}`
    } else if (t === 1) {
        return `${t} ${name}`
    } else {
        return `${t} ${name}s`
    }
}
