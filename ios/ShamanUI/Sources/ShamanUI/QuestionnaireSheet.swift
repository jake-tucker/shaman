import ShamanCore
import SwiftUI

/// A Questionnaire the open Session presents: full height, one Question to a page, each choice a card with its
/// description and its "Recommended" badge, a field where the Question takes a written answer, "Next" and then "Send
/// answers", and "Decline" in the corner. There is no review page. What the reader has half given is the screen's
/// own, kept while the Session's screen stays open, so swiped away and reviewed it returns as it was left. It takes no
/// tap for a moment after it appears, and answers sent dim it until the Server answers.
struct QuestionnaireSheet: View {
  let questionnaire: PendingQuestionnaire
  let model: SessionModel
  @Environment(\.interventionArming) private var arming
  @State private var armed = false
  @State private var detent = PresentationDetent.large

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
              QuestionPage(question: question, questionnaire: questionnaire, draft: draft, model: model)
                .id(draft.page)
            }
          }
          .frame(maxWidth: .infinity, alignment: .leading)
          .padding(20)
        }
        .scrollDismissesKeyboard(.interactively)
        QuestionnaireFoot(questionnaire: questionnaire, question: question, draft: draft, model: model)
      }
      .navigationTitle(question?.position ?? "")
      .navigationBarTitleDisplayMode(.inline)
      .protoSheetStack()
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button(role: .destructive) {
            Task { await model.decline(questionnaire) }
          } label: {
            if ProtoS.shared.symbols {
              Label(questionnaire.decline, systemImage: "xmark")
            } else {
              Text(verbatim: questionnaire.decline)
            }
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
    .protoSheet("questionnaire", places: true)
    // PROTOTYPE #718: sat like a laptop, on the half below the fold, as an Approval's sheet lands.
    .presentationDetents(ProtoL.shared.questionnaireDetents, selection: $detent)
    .onChange(of: ProtoL.shared.questionnaireDetents, initial: true) { old, detents in
      protoLog("questionnaire detents \(detents.count) was \(old.count)")
      // PROTOTYPE #719: left alone, a sheet already up keeps the detent it stands at while that is still offered.
      if ProtoS.shared.questionnaireKeeps, old != detents {
        if !detents.contains(detent) { detent = .large }
        return
      }
      detent = detents.first { $0 != .large } ?? .large
    }
    .onChange(of: detent, initial: true) { _, detent in
      protoLog("questionnaire detent \(detent == .large ? "large" : "lower")")
    }
    .presentationDragIndicator(.visible)
    .presentationBackground(Color(.systemBackground))
    .interactiveDismissDisabled(sending)
    .task(id: questionnaire.reference) {
      armed = false
      try? await Task.sleep(for: arming)
      armed = !Task.isCancelled
    }
  }
}

/// One Question's page: its heading over it, its choices as cards, and the field that takes a written answer.
private struct QuestionPage: View {
  let question: PendingQuestion
  let questionnaire: PendingQuestionnaire
  let draft: QuestionnaireDraft
  let model: SessionModel

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
