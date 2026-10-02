// The join and the Markdown, ported from mlx-lab's SpeakerTranscript.

import FluidAudio
import Foundation

struct Line: Sendable {
    let start: Double
    let end: Double
    let text: String
}

struct Turn {
    var speaker: String
    var start: Double
    var end: Double
    var text: String
}

/// Give each line the speaker who held the most talking time during it. A line
/// can fall in a gap, because the speaker detector drops quiet audio, so then
/// take the nearest segment within `snap` seconds.
func speaker(for line: Line, in segments: [TimedSpeakerSegment], snap: Double = 3.0) -> String? {
    var totals: [String: Double] = [:]
    for segment in segments {
        let overlap =
            min(line.end, Double(segment.endTimeSeconds))
            - max(line.start, Double(segment.startTimeSeconds))
        if overlap > 0 { totals[segment.speakerId, default: 0] += overlap }
    }
    if let best = totals.max(by: { $0.value < $1.value })?.key { return best }

    func distance(_ segment: TimedSpeakerSegment) -> Double {
        min(
            abs(Double(segment.startTimeSeconds) - line.end),
            abs(Double(segment.endTimeSeconds) - line.start))
    }
    guard let nearest = segments.min(by: { distance($0) < distance($1) }) else { return nil }
    return distance(nearest) <= snap ? nearest.speakerId : nil
}

/// Talk-time order, so the busiest voice is Speaker 1.
func speakerNames(_ segments: [TimedSpeakerSegment]) -> [String: String] {
    var talkTime: [String: Double] = [:]
    for segment in segments {
        talkTime[segment.speakerId, default: 0] += Double(segment.durationSeconds)
    }
    var names: [String: String] = [:]
    for (index, id) in talkTime.sorted(by: { $0.value > $1.value }).map(\.key).enumerated() {
        names[id] = "Speaker \(index + 1)"
    }
    return names
}

/// Join neighbouring lines from one person into readable turns. A silence
/// longer than `gapLimit` breaks the turn even when the voice has not changed:
/// the two halves are separate remarks, not one sentence.
func merge(_ lines: [Line], gapLimit: Double = 2.0, speakerOf: (Line) -> String) -> [Turn] {
    var turns: [Turn] = []
    for line in lines {
        let text = line.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.isEmpty { continue }
        let who = speakerOf(line)
        if let last = turns.last, last.speaker == who, line.start - last.end <= gapLimit {
            turns[turns.count - 1].end = line.end
            turns[turns.count - 1].text += " " + text
        } else {
            turns.append(Turn(speaker: who, start: line.start, end: line.end, text: text))
        }
    }
    return turns
}

func clock(_ seconds: Double) -> String {
    let total = Int(max(0, seconds))
    let hours = total / 3600
    if hours > 0 {
        return String(format: "%d:%02d:%02d", hours, (total % 3600) / 60, total % 60)
    }
    return String(format: "%02d:%02d", total / 60, total % 60)
}

/// `guessedSpeakers` is set only when the run did not pin a count, and the
/// header says so because a guessed count is the one the reader should doubt.
func markdown(_ turns: [Turn], guessedSpeakers: Int?) -> String {
    var out = ""
    if let guessedSpeakers { out += "Speakers guessed: \(guessedSpeakers)\n\n" }
    out += turns.map { "[\(clock($0.start))] \($0.speaker)\n\($0.text)" }
        .joined(separator: "\n\n")
    return out + "\n"
}
