import ShamanCore
import SwiftUI

/// A Questionnaire the open Session presents: full height, one Question to a page, each choice a card with its
/// description and its "Recommended" badge, a field where the Question takes a written answer, "Next" and then "Send
/// answers", and "Decline" in the corner. There is no review page. What the reader has half given is the screen's
/// own, kept while the Session's screen stays open, so swiped away and reviewed it returns as it was left. It takes no
/// tap for a moment after it appears, and answers sent dim it until the Server answers.
///
/// Sat like a laptop, as `pose` says, it is offered one more height, the half below the fold, and stands there,
/// presented in the pose or up as the pose is entered, unless one of its fields holds the keyboard; it goes to full
/// height as a field takes the keyboard, and as the pose is left. It is the one thing a change of pose moves on
/// purpose, as the reader never chose its height; within the pose it is left where the reader puts it.
struct QuestionnaireSheet: View {
  let questionnaire: PendingQuestionnaire
  let model: SessionModel
  /// How the window stands, which the sheet reads the fold from.
  let pose: Pose?
  @Environment(\.interventionArming) private var arming
  @State private var armed = false
  /// The page whose field holds the keyboard, and none where none does.
  @FocusState private var writing: Int?
  /// The height the sheet stands at, of those it is offered.
  @State private var height: PresentationDetent

  init(questionnaire: PendingQuestionnaire, model: SessionModel, pose: Pose?) {
    self.questionnaire = questionnaire
    self.model = model
    self.pose = pose
    // Presented in the pose, where no field of it can hold the keyboard yet, it begins on the half below the fold.
    _height = State(initialValue: pose?.fold?.halfBelow ?? .large)
  }

  var body: some View {
    let draft = model.draft(of: questionnaire)
    let sending = model.answering.contains(questionnaire.reference)
    let question = questionnaire.questions.indices.contains(draft.page) ? questionnaire.questions[draft.page] : nil
    NavigationStack {
      VStack(spacing: 0) {
        if let banner = model.banner {
          Banner(text: banner)
            .padding(.top, 4)
            .onTapGesture { model.dismissBanner() }
            .accessibilityAddTraits(.isButton)
            .accessibilityHint("Dismisses this")
        }
        QuestionnairePage(
          questionnaire: questionnaire, question: question, draft: draft, model: model, writing: $writing
        )
        .id(draft.page)
        QuestionnaireFoot(questionnaire: questionnaire, question: question, draft: draft, model: model)
      }
      .navigationTitle(question?.position ?? "")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button(role: .destructive) {
            Task { await model.decline(questionnaire) }
          } label: {
            Text(verbatim: questionnaire.decline)
          }
          .accessibilityIdentifier("questionnaire.decline")
        }
      }
    }
    .disabled(!armed || sending)
    .opacity(sending ? 0.5 : 1)
    .overlay {
      if sending {
        ProgressView()
          .accessibilityIdentifier("questionnaire.sending")
      }
    }
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("questionnaire.sheet")
    .presentationDetents(Self.heights(about: pose?.fold), selection: $height)
    .onChange(of: pose?.fold) { old, fold in
      height = Self.height(height, from: old, to: fold, writing: writing != nil)
    }
    .onChange(of: writing) { _, writing in
      if writing != nil {
        height = .large
      }
    }
    .presentationDragIndicator(.visible)
    .presentationBackground(Color(.systemBackground))
    .interactiveDismissDisabled(sending)
    // It pushes nothing, and "Decline" in words has no place in a bar down its side, which would only narrow it.
    .toolbarVerticalBehavior(.disabled)
    .task(id: questionnaire.reference) {
      armed = false
      try? await Task.sleep(for: arming)
      armed = !Task.isCancelled
    }
  }
}

extension QuestionnaireSheet {
  /// The heights the sheet is offered: full height, and the half below `fold` while there is one.
  fileprivate static func heights(about fold: Fold?) -> Set<PresentationDetent> {
    fold.map { [$0.halfBelow, .large] } ?? [.large]
  }

  /// The height the sheet stands at, standing at `height`, as the fold changes `from` one `to` another: on the half below
  /// as the pose is entered, unless a field is `writing`; at full height as it is left; and within the pose where it
  /// stood, on the half below the fold read again where it stood on the half.
  fileprivate static func height(
    _ height: PresentationDetent, from old: Fold?, to fold: Fold?, writing: Bool
  ) -> PresentationDetent {
    guard let fold else { return .large }
    guard let old else { return writing ? .large : fold.halfBelow }
    return height == old.halfBelow ? fold.halfBelow : height
  }
}

extension Fold {
  /// The half below the fold as a height a sheet stands at, its top 8 points past the band kept for the fold, so that
  /// what stands in its corner is clear of the band. A height detent stands 42 points taller than it is asked, the
  /// window's bottom safe area and 8 more, so it is asked for that half less 50.
  fileprivate var halfBelow: PresentationDetent {
    .height(max(0, window - bottom - 50))
  }
}

/// A page of the sheet, scrolled by itself: the Questionnaire's note over its Question. Each page has a scroll view of
/// its own, so that it stands at its head as it is come to and not as far down as the page before it was.
private struct QuestionnairePage: View {
  let questionnaire: PendingQuestionnaire
  let question: PendingQuestion?
  let draft: QuestionnaireDraft
  let model: SessionModel
  let writing: FocusState<Int?>.Binding
  /// Whether the reader is moving the page, by a drag or by what is left of one.
  @State private var dragged = false

  var body: some View {
    ScrollViewReader { page in
      ScrollView {
        VStack(alignment: .leading, spacing: 12) {
          if let note = questionnaire.note {
            Label {
              Text(verbatim: note)
            } icon: {
              Image(systemName: "exclamationmark.triangle")
            }
            .font(.footnote)
            .foregroundStyle(.orange)
            .accessibilityIdentifier("questionnaire.note")
          }
          if let question {
            QuestionView(
              question: question, questionnaire: questionnaire, draft: draft, model: model, writing: writing)
          }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(20)
      }
      .scrollDismissesKeyboard(.interactively)
      .onScrollPhaseChange { _, phase in
        dragged = phase != .idle && phase != .animating
      }
      .onChange(of: draft.written) { _, written in
        // Emptied by a choice taken in its place, it has nothing to keep in view, and the choice has.
        if !written.isEmpty { keepFieldInView(page) }
      }
      .onGeometryChange(for: CGFloat.self) {
        $0.size.height
      } action: { _ in
        keepFieldInView(page)
      }
    }
  }

  /// Scrolls the page to its field's foot while the field holds the keyboard, as what is written grows it and as the
  /// room for the page changes: the system scrolls to a field once, as it takes the keyboard. At other times the page
  /// is left where the reader has it, or it would go to its foot as the keyboard is put down. So it is while the reader
  /// moves it: a drag that takes the keyboard down changes the room all the way, and the page scrolled under it would
  /// end it with the keyboard still up.
  private func keepFieldInView(_ page: ScrollViewProxy) {
    guard writing.wrappedValue == draft.page, !dragged else { return }
    page.scrollTo(QuestionView.field, anchor: .bottom)
  }
}

/// One Question: its heading over it, its choices as cards, and the field that takes a written answer.
private struct QuestionView: View {
  let question: PendingQuestion
  let questionnaire: PendingQuestionnaire
  let draft: QuestionnaireDraft
  let model: SessionModel
  let writing: FocusState<Int?>.Binding

  /// What the field is known by, for the page to be scrolled to it.
  static let field = "questionnaire.field"

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      if let heading = question.heading {
        Text(verbatim: heading)
          .font(.caption.weight(.semibold))
          .textCase(.uppercase)
          .foregroundStyle(.secondary)
          .accessibilityIdentifier("questionnaire.heading")
      }
      Text(verbatim: question.text)
        .font(.title3.weight(.semibold))
        .accessibilityAddTraits(.isHeader)
        .accessibilityIdentifier("questionnaire.question")
      if let hint = question.hint {
        Text(verbatim: hint)
          .font(.footnote)
          .foregroundStyle(.secondary)
          .accessibilityIdentifier("questionnaire.hint")
      }
      ForEach(question.choices, id: \.choice) { card in
        ChoiceCardView(card: card, multiple: question.multiple, taken: draft.taken(card.choice)) {
          model.changeDraft(of: questionnaire) { $0.choose(card.choice, of: question) }
        }
      }
      if let field = question.field {
        AnswerFieldView(field: field, text: written)
          .focused(writing, equals: draft.page)
          .id(Self.field)
      }
      if draft.readsSecret, let secret = question.secret {
        Label {
          Text(verbatim: secret)
        } icon: {
          Image(systemName: "lock.fill")
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("questionnaire.secret")
      }
    }
  }

  /// What the field holds, each change handed to the draft.
  private var written: Binding<String> {
    Binding(
      get: { model.draft(of: questionnaire).written },
      set: { text in model.changeDraft(of: questionnaire) { $0.write(text, of: question) } })
  }
}

/// One choice as a card: its label and badge over its description, and whether it is taken. A Question taking several
/// marks them with checkboxes, one taking one with a radio.
private struct ChoiceCardView: View {
  let card: ChoiceCard
  let multiple: Bool
  let taken: Bool
  let choose: () -> Void

  var body: some View {
    Button(action: choose) {
      HStack(spacing: 12) {
        VStack(alignment: .leading, spacing: 3) {
          HStack(spacing: 8) {
            Text(verbatim: card.label)
              .fontWeight(.medium)
            if let badge = card.badge {
              Text(verbatim: badge)
                .font(.caption2)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(.tint.opacity(0.15), in: Capsule())
                .accessibilityIdentifier("questionnaire.badge")
            }
          }
          if let description = card.description {
            Text(verbatim: description)
              .font(.footnote)
              .foregroundStyle(.secondary)
          }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        Image(systemName: symbol)
          .foregroundStyle(taken ? Color.accentColor : .secondary)
          .imageScale(.large)
          .accessibilityHidden(true)
      }
      .padding(12)
      .background(
        taken ? Color.accentColor.opacity(0.12) : Color(.secondarySystemBackground),
        in: RoundedRectangle(cornerRadius: 12)
      )
      .contentShape(RoundedRectangle(cornerRadius: 12))
    }
    .buttonStyle(.plain)
    .accessibilityElement(children: .combine)
    .accessibilityAddTraits(taken ? .isSelected : [])
    .accessibilityIdentifier("questionnaire.choice")
  }

  private var symbol: String {
    switch (multiple, taken) {
    case (true, true): "checkmark.square.fill"
    case (true, false): "square"
    case (false, true): "checkmark.circle.fill"
    case (false, false): "circle"
    }
  }
}

/// The field that takes a written answer: a secure one for a secret.
private struct AnswerFieldView: View {
  let field: AnswerField
  let text: Binding<String>

  var body: some View {
    Group {
      if field.secure {
        SecureField(text: text) { Text(verbatim: field.label) }
      } else {
        TextField(text: text, axis: .vertical) { Text(verbatim: field.label) }
          .lineLimit(1...6)
      }
    }
    .textFieldStyle(.roundedBorder)
    .autocorrectionDisabled(field.secure)
    .textInputAutocapitalization(field.secure ? .never : .sentences)
    .accessibilityIdentifier("questionnaire.field")
  }
}

/// The sheet's foot: "Back" from every page but the first, "Skip" where the Question is not required, and "Next",
/// which on the last page is "Send answers" and is held until a required Question is answered. "Send answers" alone
/// sends: "Skip" on the last page leaves its Question unanswered and stays.
private struct QuestionnaireFoot: View {
  let questionnaire: PendingQuestionnaire
  let question: PendingQuestion?
  let draft: QuestionnaireDraft
  let model: SessionModel

  var body: some View {
    HStack(spacing: 10) {
      if draft.page > 0 {
        Button {
          model.changeDraft(of: questionnaire) { $0.back() }
        } label: {
          Text(verbatim: questionnaire.back)
        }
        .buttonStyle(.bordered)
        .accessibilityIdentifier("questionnaire.back")
      }
      if let skip = question?.skip {
        Button {
          model.changeDraft(of: questionnaire) { $0.skip(questionnaire) }
        } label: {
          Text(verbatim: skip)
        }
        .buttonStyle(.bordered)
        .accessibilityIdentifier("questionnaire.skip")
      }
      Button {
        go { $0.next(questionnaire) }
      } label: {
        Text(verbatim: draft.onLast(questionnaire) ? questionnaire.send : questionnaire.next)
          .frame(maxWidth: .infinity)
      }
      .buttonStyle(.borderedProminent)
      .disabled(question != nil && !draft.goesOn(questionnaire))
      .accessibilityIdentifier("questionnaire.next")
    }
    .controlSize(.large)
    .padding(.horizontal, 20)
    .padding(.vertical, 12)
  }

  /// Moves the draft on as `step` does, sending the answers where "Send answers" was tapped.
  private func go(_ step: (inout QuestionnaireDraft) -> Bool) {
    if model.changeDraft(of: questionnaire, step) {
      Task { await model.sendAnswers(questionnaire) }
    }
  }
}
