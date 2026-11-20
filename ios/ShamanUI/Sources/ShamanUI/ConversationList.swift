import ShamanCore
import SwiftUI
import UIKit

/// Where each row of a conversation stands, from the heights its rows have been measured at and a guess for those not
/// yet drawn: the arithmetic the conversation's layout runs on, apart from UIKit so that it can be tested.
///
/// A row measured at a height other than the one it stood at moves every row below it. What stands at the foot of what
/// the reader sees is kept still: where the row ended above it, as a row does that is first drawn at the top of what
/// they see while they scroll up, `measured` says how far to scroll to keep it so; where the row runs on past it, as a
/// long answer the reader is reading inside does while it streams, what they see is the row's own head, which stands.
struct ConversationGeometry {
  /// The space between rows, and above the first and below the last.
  static let spacing: CGFloat = 12
  static let margin: CGFloat = 8
  /// What a row not yet drawn is guessed to measure.
  static let guess: CGFloat = 120
  /// How long a row must have stood shorter than it was before it is taken to be so: Textual draws a Message empty,
  /// then grows it into place over a few passes, as it is drawn and now and then again while it stands, and a row
  /// shrinking for that would move what stands below it and back.
  static let settling: TimeInterval = 0.3

  private(set) var ids: [String] = []
  private var indices: [String: Int] = [:]
  private var heights: [String: CGFloat] = [:]
  /// The rows reported shorter than they stand, with what they measure and since when.
  private var shrinking: [String: (height: CGFloat, since: TimeInterval)] = [:]
  private var tops: [CGFloat] = []
  private(set) var height: CGFloat = 0

  /// Takes `ids` as the rows, in order, keeping what was measured of those still among them.
  mutating func take(_ ids: [String]) {
    guard ids != self.ids else { return }
    self.ids = ids
    indices = Dictionary(ids.enumerated().map { ($1, $0) }, uniquingKeysWith: { first, _ in first })
    heights = heights.filter { indices[$0.key] != nil }
    shrinking = shrinking.filter { indices[$0.key] != nil }
    place()
  }

  /// Forgets every height measured, as rows drawn at another width measure anew.
  mutating func forget() {
    heights = [:]
    shrinking = [:]
    place()
  }

  /// The row at `index`'s frame across `width`.
  func frame(of index: Int, width: CGFloat) -> CGRect {
    CGRect(x: 0, y: tops[index], width: width, height: height(of: ids[index]))
  }

  func height(of id: String) -> CGFloat {
    heights[id] ?? Self.guess
  }

  /// Whether the row `id` has been measured.
  func measured(_ id: String) -> Bool {
    heights[id] != nil
  }

  /// The rows standing at least partly between `top` and `bottom`.
  func rows(between top: CGFloat, and bottom: CGFloat) -> Range<Int> {
    guard !ids.isEmpty else { return 0..<0 }
    // The first row whose foot is below `top`, and the first whose head is below `bottom`.
    let first = partition { tops[$0] + height(of: ids[$0]) > top }
    let end = partition { tops[$0] >= bottom }
    return first..<max(first, end)
  }

  /// What the row `id` measures, as drawn at `time`: the height to stand it at now, where there is one. A row not yet
  /// measured stands at what it first measures, and one that grows stands taller at once; one that measures shorter
  /// stands as it is until it has measured so for `settling`, which `settled(at:)` answers.
  mutating func reported(_ id: String, _ height: CGFloat, at time: TimeInterval) -> CGFloat? {
    guard indices[id] != nil else { return nil }
    guard let standing = heights[id] else { return height }
    if height < standing - 0.25 {
      if shrinking[id]?.height != height {
        shrinking[id] = (height, time)
      }
      return nil
    }
    shrinking[id] = nil
    return height > standing + 0.25 ? height : nil
  }

  /// When the next row reported shorter will have measured so for `settling`.
  var nextSettling: TimeInterval? {
    shrinking.values.map { $0.since + Self.settling }.min()
  }

  /// The rows that have measured shorter for `settling` by `time`, and the heights to stand them at.
  mutating func settled(at time: TimeInterval) -> [(id: String, height: CGFloat)] {
    let due = shrinking.filter { time >= $0.value.since + Self.settling }
    for id in due.keys {
      shrinking[id] = nil
    }
    return due.map { ($0.key, $0.value.height) }.sorted { indices[$0.id] ?? 0 < indices[$1.id] ?? 0 }
  }

  /// Takes `height` as what the row `id` measures, answering how far what stands at `foot` moved: the row's change,
  /// where it ended above `foot`, and nothing otherwise.
  @discardableResult
  mutating func measured(_ id: String, at height: CGFloat, foot: CGFloat) -> CGFloat {
    guard let index = indices[id] else { return 0 }
    let was = self.height(of: id)
    let change = height - was
    guard abs(change) > 0.25 else { return 0 }
    heights[id] = height
    let above = tops[index] + was <= foot
    place()
    return above ? change : 0
  }

  private mutating func place() {
    tops = []
    tops.reserveCapacity(ids.count)
    var y = Self.margin
    for id in ids {
      tops.append(y)
      y += height(of: id) + Self.spacing
    }
    height = ids.isEmpty ? 0 : y - Self.spacing + Self.margin
  }

  /// The first index for which `isAfter` holds, in rows ordered so that it holds from some index on.
  private func partition(_ isAfter: (Int) -> Bool) -> Int {
    var low = 0
    var high = ids.count
    while low < high {
      let middle = (low + high) / 2
      if isAfter(middle) {
        high = middle
      } else {
        low = middle + 1
      }
    }
    return low
  }
}

/// One row of an open conversation as its list holds it: an item, or a Prompt queued at the foot.
enum ConversationRow: Equatable {
  case item(ConversationItem)
  case queued(QueuedPrompt)

  var id: String {
    switch self {
    case .item(let item): "item:\(item.id)"
    case .queued(let queued): "queued:\(queued.prompt)"
    }
  }
}

/// An open conversation's rows in a collection view laid out by `ConversationGeometry`, standing at its foot when it
/// opens and following it as it grows while the reader stands there.
///
/// SwiftUI's lazy stacks place a row they have yet to draw by a guess at it, and correct the guess where the row is
/// drawn without keeping what the reader sees still, so scrolling up through rows not yet drawn jumps, and anchored at
/// the foot a long conversation opens on a guess past its end. A `List` does better but forgets what it measured of a
/// row now and then, and a plain stack draws every row at once, and again at every change, which hundreds of Markdown
/// Messages cannot afford. So the rows are drawn as they come into sight, as in any of those, and each row's height is
/// kept once measured: a row is measured as it is first drawn, and says each height it comes to after that, from
/// SwiftUI. Where a row ending above the foot of what the reader sees comes to stand taller or shorter, as each does
/// when it is first drawn and an Agent's Markdown does as it is parsed, the list scrolls by as much, so what stands at
/// the foot of the screen stays where it stands.
///
/// At the foot it stays at the foot: each change in what the rows measure, or in the room the list has, scrolls it to
/// the foot again. Any scroll away from the foot leaves it, a drag, VoiceOver's or the system's alike, and coming to
/// rest close to the foot again, or sending, takes it back.
struct ConversationList<Row: View>: UIViewRepresentable {
  let rows: [ConversationRow]
  /// How many Prompts the composer has sent, each of which takes the list back to its foot.
  let sent: Int
  @ViewBuilder let row: (ConversationRow) -> Row

  func makeCoordinator() -> ConversationFollower {
    ConversationFollower()
  }

  func makeUIView(context: Context) -> FootCollectionView {
    let view = context.coordinator.makeView()
    view.accessibilityIdentifier = "session.conversation"
    return view
  }

  func updateUIView(_ view: FootCollectionView, context: Context) {
    let follower = context.coordinator
    let row = row
    follower.configuration = { [weak follower] held in
      // Each row its own identity, so that what one row's views hold never stands in another's; measured at its own
      // height, which it says each time it changes, and drawn from the top of its cell.
      UIHostingConfiguration {
        row(held)
          .fixedSize(horizontal: false, vertical: true)
          .onGeometryChange(for: CGFloat.self) {
            ceil($0.size.height)
          } action: { height in
            follower?.report(held.id, height)
          }
          .frame(maxHeight: .infinity, alignment: .top)
          .id(held.id)
      }
      .margins(.all, 0)
    }
    if sent != follower.sent {
      follower.sent = sent
      follower.following = true
    }
    follower.take(rows)
  }
}

/// What an open conversation's list follows: the rows it holds, where each stands, and whether the reader stands at
/// the foot.
@MainActor final class ConversationFollower: NSObject, UICollectionViewDelegate {
  /// How near the foot a reader at rest stands at it.
  static let near: CGFloat = 48

  weak var view: FootCollectionView?
  var dataSource: UICollectionViewDiffableDataSource<Int, String>?
  var configuration: ((ConversationRow) -> any UIContentConfiguration)?
  var geometry = ConversationGeometry()
  private(set) var rows: [String: ConversationRow] = [:]
  var sent = 0
  /// Whether the reader stands at the foot, so that the list follows it.
  var following = true {
    didSet {
      if following, !oldValue {
        view?.setNeedsLayout()
      }
    }
  }
  private var settling: Task<Void, Never>?
  private let clock: () -> TimeInterval

  init(clock: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
    self.clock = clock
  }

  /// The collection view the list scrolls in, laid out by this follower's geometry.
  func makeView() -> FootCollectionView {
    let view = FootCollectionView(frame: .zero, collectionViewLayout: ConversationLayout(follower: self))
    view.follower = self
    view.backgroundColor = .clear
    view.alwaysBounceVertical = true
    view.keyboardDismissMode = .interactive
    // A row is no target of its own: taps go to what it holds.
    view.allowsSelection = false
    view.contentInsetAdjustmentBehavior = .always
    // A row says each height it comes to itself; UIKit measures only a row not yet measured.
    view.selfSizingInvalidation = .disabled
    view.delegate = self
    self.view = view
    let registration = UICollectionView.CellRegistration<RowCell, String> { [weak self] cell, _, id in
      guard let self, let row = self.rows[id] else { return }
      cell.measured = { [weak self] in self?.geometry.measured(id) ?? false }
      cell.contentConfiguration = self.configuration?(row)
    }
    dataSource = UICollectionViewDiffableDataSource(collectionView: view) { view, index, id in
      view.dequeueConfiguredReusableCell(using: registration, for: index, item: id)
    }
    return view
  }

  /// Takes `rows` in, drawing again those that changed.
  func take(_ rows: [ConversationRow]) {
    guard let dataSource else { return }
    let ids = rows.map(\.id)
    let changed = rows.filter { row in self.rows[row.id].map { $0 != row } ?? false }.map(\.id)
    self.rows = Dictionary(rows.map { ($0.id, $0) }, uniquingKeysWith: { _, last in last })
    var snapshot = dataSource.snapshot()
    if snapshot.itemIdentifiers != ids {
      geometry.take(ids)
      snapshot = NSDiffableDataSourceSnapshot()
      snapshot.appendSections([0])
      snapshot.appendItems(ids)
    }
    if !changed.isEmpty {
      snapshot.reconfigureItems(changed)
    }
    dataSource.apply(snapshot, animatingDifferences: false)
    view?.setNeedsLayout()
  }

  /// Takes what the row `id` says it measures now.
  func report(_ id: String, _ height: CGFloat) {
    if let height = geometry.reported(id, height, at: clock()) {
      stand(id, at: height)
    }
    settleLater()
  }

  /// Stands the row `id` at `height`, keeping still what stands at the foot of what the reader sees.
  func stand(_ id: String, at height: CGFloat) {
    guard let view else {
      geometry.measured(id, at: height, foot: 0)
      return
    }
    let moved = geometry.measured(id, at: height, foot: view.visibleFoot)
    let context = UICollectionViewLayoutInvalidationContext()
    // At the foot the list goes back to the foot; above it, what the reader sees stays where it stands.
    if moved != 0, !following {
      context.contentOffsetAdjustment = CGPoint(x: 0, y: moved)
    }
    view.collectionViewLayout.invalidateLayout(with: context)
    view.setNeedsLayout()
  }

  /// Stands each row that has measured shorter for long enough at what it measures, once it has.
  private func settleLater() {
    guard settling == nil, let next = geometry.nextSettling else { return }
    settling = Task { [weak self] in
      try? await Task.sleep(for: .seconds(max(0, next - (self?.clock() ?? next))))
      guard let self, !Task.isCancelled else { return }
      self.settling = nil
      for (id, height) in self.geometry.settled(at: self.clock()) {
        self.stand(id, at: height)
      }
      self.settleLater()
    }
  }

  func scrollViewWillBeginDragging(_ scrollView: UIScrollView) {
    following = false
  }

  func scrollViewDidEndDragging(_ scrollView: UIScrollView, willDecelerate decelerate: Bool) {
    if !decelerate {
      rested()
    }
  }

  func scrollViewDidEndDecelerating(_ scrollView: UIScrollView) {
    rested()
  }

  func scrollViewDidEndScrollingAnimation(_ scrollView: UIScrollView) {
    rested()
  }

  func scrollViewDidScroll(_ scrollView: UIScrollView) {
    view?.scrolled()
  }

  func scrollViewShouldScrollToTop(_ scrollView: UIScrollView) -> Bool {
    following = false
    return true
  }

  /// The reader at rest stands at the foot where they are near it.
  private func rested() {
    following = view.map { $0.distanceToFoot <= Self.near } ?? false
  }
}

/// A row's cell. A row not yet measured is as tall as what it hosts measures across the list's width; one measured
/// stands as tall as it was, and says after that what it comes to.
final class RowCell: UICollectionViewCell {
  /// Whether the row this cell draws has been measured.
  var measured: () -> Bool = { false }

  override func preferredLayoutAttributesFitting(
    _ attributes: UICollectionViewLayoutAttributes
  ) -> UICollectionViewLayoutAttributes {
    guard !measured() else { return attributes }
    let fitted = attributes.copy() as! UICollectionViewLayoutAttributes
    let size = contentView.systemLayoutSizeFitting(
      CGSize(width: attributes.size.width, height: UIView.layoutFittingCompressedSize.height),
      withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel)
    fitted.size.height = ceil(size.height)
    return fitted
  }
}

/// The collection view an open conversation scrolls in, which goes back to its foot each time it is laid out while
/// the reader stands there, and leaves it on any scroll away.
final class FootCollectionView: UICollectionView {
  weak var follower: ConversationFollower?
  /// Where the list last went back to its foot, so that a scroll from there that it did not make is the reader's.
  private var pinned: CGFloat?
  private var pinning = false

  /// How far above its foot the list stands.
  var distanceToFoot: CGFloat {
    max(0, foot - contentOffset.y)
  }

  /// The depth of the foot of what the reader sees, in what the list holds.
  var visibleFoot: CGFloat {
    contentOffset.y + bounds.height - adjustedContentInset.bottom
  }

  private var foot: CGFloat {
    max(-adjustedContentInset.top, contentSize.height + adjustedContentInset.bottom - bounds.height)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    guard follower?.following == true, !isTracking, !isDecelerating else { return }
    let foot = foot
    if abs(contentOffset.y - foot) > 0.5 {
      pinning = true
      contentOffset.y = foot
      pinning = false
    }
    pinned = foot
  }

  /// Leaves the foot where the list moved up, further than a reader at rest near the foot stands from it, from where
  /// it went back to it and from the foot as it stands now, by any hand but its own: the system scrolling to a row
  /// VoiceOver or the keyboard moved to, or one the app asked for. The system easing the insets as a screen is pushed
  /// moves it a few points, growing below moves nothing, and shrinking only brings the list up with its foot.
  func scrolled() {
    let near = ConversationFollower.near
    guard !pinning, let follower, follower.following, let pinned, contentOffset.y < pinned - near,
      distanceToFoot > near
    else { return }
    follower.following = false
  }

  /// VoiceOver's three-finger scroll: a page at a time, leaving the foot as any scroll does.
  override func accessibilityScroll(_ direction: UIAccessibilityScrollDirection) -> Bool {
    let page = bounds.height - adjustedContentInset.top - adjustedContentInset.bottom
    let top = -adjustedContentInset.top
    var to: CGFloat
    switch direction {
    case .up: to = min(foot, contentOffset.y + page * 0.9)
    case .down: to = max(top, contentOffset.y - page * 0.9)
    default: return false
    }
    // Paged to within a few lines of the foot, the reader is at it.
    if direction == .up, foot - to <= ConversationFollower.near {
      to = foot
    }
    guard abs(to - contentOffset.y) > 0.5 else { return false }
    follower?.following = to >= foot - 0.5
    setContentOffset(CGPoint(x: contentOffset.x, y: to), animated: false)
    UIAccessibility.post(notification: .pageScrolled, argument: nil)
    return true
  }
}

/// The layout of an open conversation: its rows one above another across its width, where `ConversationGeometry`
/// stands them.
final class ConversationLayout: UICollectionViewLayout {
  private weak var follower: ConversationFollower?
  private var width: CGFloat = 0

  init(follower: ConversationFollower) {
    self.follower = follower
    super.init()
  }

  required init?(coder: NSCoder) {
    fatalError("init(coder:) has not been implemented")
  }

  private var geometry: ConversationGeometry {
    follower?.geometry ?? ConversationGeometry()
  }

  override func prepare() {
    super.prepare()
    let width = collectionView?.bounds.width ?? 0
    if width != self.width {
      // Drawn across another width, every row measures anew.
      if self.width != 0 {
        follower?.geometry.forget()
      }
      self.width = width
    }
  }

  override var collectionViewContentSize: CGSize {
    CGSize(width: width, height: geometry.height)
  }

  override func layoutAttributesForElements(in rect: CGRect) -> [UICollectionViewLayoutAttributes]? {
    let geometry = geometry
    return geometry.rows(between: rect.minY, and: rect.maxY).map { attributes(at: $0, in: geometry) }
  }

  override func layoutAttributesForItem(at indexPath: IndexPath) -> UICollectionViewLayoutAttributes? {
    let geometry = geometry
    guard geometry.ids.indices.contains(indexPath.item) else { return nil }
    return attributes(at: indexPath.item, in: geometry)
  }

  private func attributes(at index: Int, in geometry: ConversationGeometry) -> UICollectionViewLayoutAttributes {
    let attributes = UICollectionViewLayoutAttributes(forCellWith: IndexPath(item: index, section: 0))
    attributes.frame = geometry.frame(of: index, width: width)
    return attributes
  }

  override func shouldInvalidateLayout(forBoundsChange newBounds: CGRect) -> Bool {
    newBounds.width != width
  }

  override func shouldInvalidateLayout(
    forPreferredLayoutAttributes preferred: UICollectionViewLayoutAttributes,
    withOriginalAttributes original: UICollectionViewLayoutAttributes
  ) -> Bool {
    abs(preferred.size.height - original.size.height) > 0.25
  }

  override func invalidationContext(
    forPreferredLayoutAttributes preferred: UICollectionViewLayoutAttributes,
    withOriginalAttributes original: UICollectionViewLayoutAttributes
  ) -> UICollectionViewLayoutInvalidationContext {
    let context = super.invalidationContext(
      forPreferredLayoutAttributes: preferred, withOriginalAttributes: original)
    guard let collectionView = collectionView as? FootCollectionView, let follower,
      geometry.ids.indices.contains(original.indexPath.item)
    else { return context }
    let id = geometry.ids[original.indexPath.item]
    let moved = follower.geometry.measured(id, at: preferred.size.height, foot: collectionView.visibleFoot)
    if moved != 0, !follower.following {
      context.contentOffsetAdjustment = CGPoint(x: 0, y: moved)
    }
    return context
  }
}
