import Foundation

/// What ticks, which Swift alone draws: the core hands over the instant something began, as its Server gave it, and
/// this says how long ago that was. An instant in the future reads as no time at all, since no Server's clock is
/// corrected.
enum RunningTime {
  /// How long work has run since `since`: seconds until there is a minute to say, then minutes, then hours and the
  /// minutes past them, granular enough to be seen moving.
  static func duration(since: Date, now: Date) -> String {
    let seconds = elapsed(since: since, now: now)
    if seconds < 60 {
      return "\(seconds)s"
    }
    let minutes = seconds / 60
    if minutes < 60 {
      return "\(minutes)m"
    }
    return "\(minutes / 60)h \(minutes % 60)m"
  }

  /// How long the work the bar above the composer stands for has run since `since`, as the Working Indicator says it:
  /// seconds, then minutes and zero-padded seconds, then hours, minutes and seconds, so it is seen moving every second.
  static func working(since: Date, now: Date) -> String {
    let seconds = elapsed(since: since, now: now)
    if seconds < 60 {
      return "\(seconds)s"
    }
    let minutes = seconds / 60
    let padded = { (value: Int) in value < 10 ? "0\(value)" : "\(value)" }
    if minutes < 60 {
      return "\(minutes)m \(padded(seconds % 60))s"
    }
    return "\(minutes / 60)h \(padded(minutes % 60))m \(padded(seconds % 60))s"
  }

  /// The compact time since `at`: "now" within a minute, then whole minutes, hours or days.
  static func compact(since at: Date, now: Date) -> String {
    let seconds = elapsed(since: at, now: now)
    switch seconds {
    case ..<60: return "now"
    case ..<3_600: return "\(seconds / 60)m"
    case ..<86_400: return "\(seconds / 3_600)h"
    default: return "\(seconds / 86_400)d"
    }
  }

  /// Whole seconds from `since` to `now`, and none where `since` is later.
  private static func elapsed(since: Date, now: Date) -> Int {
    max(0, Int(now.timeIntervalSince(since)))
  }
}
