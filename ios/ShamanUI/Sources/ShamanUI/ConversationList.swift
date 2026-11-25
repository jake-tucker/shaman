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
  /// The space between rows but for those `gaps` names, and above the first and below the last.
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
  /// How far below the row above it each row stands, where that is not `spacing`.
  private(set) var gaps: [String: CGFloat] = [:]
  private var tops: [CGFloat] = []
  private(set) var height: CGFloat = 0

  /// Takes `ids` as the rows, in order, each as far below the one above it as `gaps` says, keeping what was measured of
  /// those still among them. An id may leave and come back naming another block of its Message: what was measured goes
  /// as a row leaves, so it stands at nothing but its own height. Answers how far the rows already held moved, at
  /// `foot`, for standing closer together or further apart: the head of the first of them ending below it.
  @discardableResult
  mutating func take(_ ids: [String], gaps: [String: CGFloat] = [:], foot: CGFloat = 0) -> CGFloat {
    guard ids != self.ids || gaps != self.gaps else { return 0 }
    let seen = partition { tops[$0] + height(of: self.ids[$0]) > foot }
    var moved: CGFloat = 0
    if gaps != self.gaps, seen < self.ids.count {
      let was = tops[seen]
      self.gaps.merge(gaps) { _, now in now }
      place()
      moved = tops[seen] - was
    }
    self.ids = ids
    self.gaps = gaps
    indices = Dictionary(ids.enumerated().map { ($1, $0) }, uniquingKeysWith: { first, _ in first })
    heights = heights.filter { indices[$0.key] != nil }
    shrinking = shrinking.filter { indices[$0.key] != nil }
    place()
    return moved
  }

  /// Forgets every height measured, as rows drawn at another width measure anew.
  mutating func forget() {
    heights = [:]
    shrinking = [:]
    place()
  }

  /// The row at `index`'s frame, across `width` from `x`.
  func frame(of index: Int, x: CGFloat = 0, width: CGFloat) -> CGRect {
    CGRect(x: x, y: tops[index], width: width, height: height(of: ids[index]))
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
    for (index, id) in ids.enumerated() {
      if index > 0 {
        y += gaps[id] ?? Self.spacing
      }
      tops.append(y)
      y += height(of: id)
    }
    height = ids.isEmpty ? 0 : y + Self.margin
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
  /// The content margins of the container the list stands in, which its rows stand by.
  let margins: EdgeInsets
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
    follower.margins = margins
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
  /// The container's content margins, which the rows stand by at each side.
  var margins = EdgeInsets() {
    didSet {
      if margins != oldValue {
        view?.collectionViewLayout.invalidateLayout()
      }
    }
  }
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
    view.registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { [weak self] (_: FootCollectionView, _) in
      guard let self else { return }
      self.place(self.geometry.ids)
    }
    let registration = UICollectionView.CellRegistration<RowCell, String> { [weak self] cell, _, id in
      guard let self, let row = self.rows[id] else { return }
      self.draw(row, in: cell)
    }
    dataSource = UICollectionViewDiffableDataSource(collectionView: view) { view, index, id in
      view.dequeueConfiguredReusableCell(using: registration, for: index, item: id)
    }
    return view
  }

  /// Takes `rows` in, drawing again those on screen that changed.
  ///
  /// The list draws them itself, as UIKit passes one over now and then where it is asked to reconfigure it: once a change
  /// of width is animated, as a column coming or going animates it, a row that left the screen and came back within it.
  func take(_ rows: [ConversationRow]) {
    guard let dataSource else { return }
    let ids = rows.map(\.id)
    self.rows = Dictionary(rows.map { ($0.id, $0) }, uniquingKeysWith: { _, last in last })
    place(ids)
    var snapshot = dataSource.snapshot()
    if snapshot.itemIdentifiers != ids {
      snapshot = NSDiffableDataSourceSnapshot()
      snapshot.appendSections([0])
      snapshot.appendItems(ids)
      dataSource.apply(snapshot, animatingDifferences: false)
    }
    for cell in view?.visibleCells ?? [] {
      drawAgain(cell)
    }
    view?.setNeedsLayout()
  }

  private func draw(_ row: ConversationRow, in cell: RowCell) {
    cell.row = row
    cell.measured = { [weak self] in self?.geometry.measured(row.id) ?? false }
    cell.contentConfiguration = configuration?(row)
  }

  /// Draws what `cell` draws again, where its row has changed since.
  private func drawAgain(_ cell: UICollectionViewCell) {
    guard let cell = cell as? RowCell, let drawn = cell.row, let row = rows[drawn.id], row != drawn else { return }
    UIView.performWithoutAnimation { draw(row, in: cell) }
  }

  func collectionView(
    _ collectionView: UICollectionView, willDisplay cell: UICollectionViewCell, forItemAt indexPath: IndexPath
  ) {
    // Drawn ahead of its coming into sight, its row may have changed in between.
    drawAgain(cell)
  }

  /// How far below the row above it each of the rows `ids` stands, where that is not the list's own gap: a block of an
  /// agent Message below another of it, by as much as Textual would have them apart at the size the text is.
  private func gaps(of ids: [String]) -> [String: CGFloat] {
    guard let view else { return [:] }
    let em = UIFont.preferredFont(forTextStyle: .body, compatibleWith: view.traitCollection).pointSize
    var gaps: [String: CGFloat] = [:]
    for (upper, lower) in zip(ids, ids.dropFirst()) {
      if let above = rows[upper], let gap = rows[lower]?.gap(below: above) {
        // Rounded down as each row's height is rounded up, so that a Message comes to what it would whole.
        gaps[lower] = floor(gap * em)
      }
    }
    return gaps
  }

  /// Stands the rows `ids` one below another, as far apart as each two are to be now.
  private func place(_ ids: [String]) {
    let stood = geometry.gaps
    let moved = geometry.take(ids, gaps: gaps(of: ids), foot: view?.visibleFoot ?? 0)
    if geometry.gaps != stood {
      invalidate(moved: moved)
    }
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
    invalidate(moved: geometry.measured(id, at: height, foot: view.visibleFoot))
  }

  /// Lays the rows out again where they stand now, what stands at the foot of what the reader sees having `moved`.
  private func invalidate(moved: CGFloat) {
    guard let view else { return }
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
    // Ended by the list going to its foot itself, which stops any scroll under way and says so before the list
    // stands at the foot, it is no reader coming to rest.
    if view?.adjusting != true {
      rested()
    }
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
  /// The row this cell draws, as it draws it.
  var row: ConversationRow?
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
/// the reader stands there.
///
/// What leaves the foot is a scroll someone asked for, never how far the list moved in a frame: the reader's drag,
/// VoiceOver's page scroll, and a scroll to an offset, a rect or a row above the foot, as the app, the keyboard or
/// focus moving asks, animated or not. When what the list holds or the room it has changes, the system scrolls it to
/// suit, asking for an offset as anyone would, and where the rows shrank, over a few frames, as far in each as it
/// pleases; that is not someone scrolling, and at the foot the list goes there itself at once, which stops it.
final class FootCollectionView: UICollectionView {
  weak var follower: ConversationFollower?
  /// Whether the list is going back to its foot itself, or the system is adjusting it to a change in what it holds
  /// or the room it has.
  private(set) var adjusting = false

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
    adjust { super.layoutSubviews() }
    guard follower?.following == true, !isTracking, !isDecelerating else { return }
    let foot = foot
    if abs(contentOffset.y - foot) > 0.5 {
      // Without animation, which also stops any scroll the system had begun toward where the foot was.
      adjust { setContentOffset(CGPoint(x: contentOffset.x, y: foot), animated: false) }
    }
  }

  override var contentSize: CGSize {
    get { super.contentSize }
    set { adjust { super.contentSize = newValue } }
  }

  override func adjustedContentInsetDidChange() {
    adjust { super.adjustedContentInsetDidChange() }
    // The room changed, so the foot moved: laid out again, the list follows it.
    setNeedsLayout()
  }

  /// Runs `body` as the list's own adjustment, or the system's, rather than someone's scroll.
  private func adjust(_ body: () -> Void) {
    let was = adjusting
    adjusting = true
    body()
    adjusting = was
  }

  /// A scroll to `offset` someone asked for leaves the foot where it goes further above it than a reader at rest near
  /// the foot stands.
  override func setContentOffset(_ offset: CGPoint, animated: Bool) {
    if !adjusting {
      leave(toward: offset.y)
    }
    super.setContentOffset(offset, animated: animated)
  }

  /// A rect above what the reader sees, brought into sight, leaves the foot.
  override func scrollRectToVisible(_ rect: CGRect, animated: Bool) {
    if !adjusting, rect.minY < contentOffset.y + adjustedContentInset.top - 0.5 {
      follower?.following = false
    }
    super.scrollRectToVisible(rect, animated: animated)
  }

  /// A row scrolled to that ends above the foot by more than a reader at rest near it stands leaves it.
  override func scrollToItem(
    at indexPath: IndexPath, at scrollPosition: UICollectionView.ScrollPosition, animated: Bool
  ) {
    if !adjusting, let row = collectionViewLayout.layoutAttributesForItem(at: indexPath),
      row.frame.maxY < contentSize.height - ConversationFollower.near
    {
      follower?.following = false
    }
    super.scrollToItem(at: indexPath, at: scrollPosition, animated: animated)
  }

  private func leave(toward offset: CGFloat) {
    if offset < foot - ConversationFollower.near {
      follower?.following = false
    }
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

/// The layout of an open conversation: its rows one above another where `ConversationGeometry` stands them, across as
/// much of its width as `ReadingSides` leaves. What is beside them scrolls it as they do.
final class ConversationLayout: UICollectionViewLayout {
  private weak var follower: ConversationFollower?
  private var width: CGFloat = 0
  /// Where the rows stand across that width.
  private var sides = ReadingSides(leading: 0, trailing: 0)

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
    let was = rowWidth
    let drawn = width != 0
    width = collectionView?.bounds.width ?? 0
    sides = ReadingSides(across: width, margins: follower?.margins ?? EdgeInsets())
    if drawn, rowWidth != was {
      // Drawn across another width, every row measures anew.
      follower?.geometry.forget()
    }
  }

  private var rowWidth: CGFloat {
    max(0, width - sides.leading - sides.trailing)
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
    let turned = collectionView?.effectiveUserInterfaceLayoutDirection == .rightToLeft
    attributes.frame = geometry.frame(of: index, x: turned ? sides.trailing : sides.leading, width: rowWidth)
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
