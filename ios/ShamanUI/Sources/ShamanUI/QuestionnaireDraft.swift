import ShamanCore

/// What a reader has half given a Questionnaire, one Question to a page: the page they are on and, for each Question,
/// the choices taken and what was written. It is the screen's own, kept while the Session's screen stays open, so a
/// Questionnaire swiped away and reviewed returns as it was left; the core is handed the answers whole when they are
/// sent.
struct QuestionnaireDraft: Equatable, Sendable {
  /// What was given one Question.
  struct Page: Equatable, Sendable {
    var choices: [String] = []
    var text = ""
    /// Whether a secret's answer was left behind on an earlier visit, so it reads "Answered (secret)" and shows
    /// nothing of what it was.
    var sealed = false

    /// Whether anything was given: a choice taken, or something written beyond blanks.
    var answered: Bool {
      !choices.isEmpty || !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
  }

  /// The page on show, from 0.
  private(set) var page = 0
  private(set) var pages: [Page]

  init(_ questionnaire: PendingQuestionnaire) {
    pages = Array(repeating: Page(), count: questionnaire.questions.count)
  }

  /// Whether the page on show is the last, whose tap sends the answers.
  func onLast(_ questionnaire: PendingQuestionnaire) -> Bool {
    page >= questionnaire.questions.count - 1
  }

  /// Whether the page on show reads "Answered (secret)".
  var sealed: Bool { pages.indices.contains(page) && pages[page].sealed }

  /// Whether `choice` shows as taken on the page on show: never a sealed secret's.
  func taken(_ choice: String) -> Bool {
    guard pages.indices.contains(page), !pages[page].sealed else { return false }
    return pages[page].choices.contains(choice)
  }

  /// What the field shows on the page on show: nothing of a sealed secret's.
  var written: String {
    guard pages.indices.contains(page), !pages[page].sealed else { return "" }
    return pages[page].text
  }

  /// Takes `choice` on the page on show of `question`, or lets it go where it was taken. Where one choice alone may
  /// be taken it replaces another, and where what is written does not stand beside a choice it is cleared.
  mutating func choose(_ choice: String, of question: PendingQuestion) {
    guard pages.indices.contains(page) else { return }
    unseal()
    var given = pages[page]
    if let taken = given.choices.firstIndex(of: choice) {
      given.choices.remove(at: taken)
    } else if question.multiple {
      given.choices.append(choice)
    } else {
      given.choices = [choice]
    }
    if let field = question.field, !field.combines {
      given.text = ""
    }
    pages[page] = given
  }

  /// Writes `text` on the page on show of `question`. Where it does not stand beside a choice, writing clears the
  /// choice taken.
  mutating func write(_ text: String, of question: PendingQuestion) {
    guard pages.indices.contains(page), text != written else { return }
    unseal()
    pages[page].text = text
    if let field = question.field, !field.combines, !text.isEmpty {
      pages[page].choices = []
    }
  }

  /// Whether the page on show of `questionnaire` lets the reader go on: a required Question holds "Next" until it is
  /// answered.
  func goesOn(_ questionnaire: PendingQuestionnaire) -> Bool {
    // A Questionnaire asking nothing is sent as it stands.
    guard questionnaire.questions.indices.contains(page), pages.indices.contains(page) else {
      return questionnaire.questions.isEmpty
    }
    return questionnaire.questions[page].skip != nil || pages[page].answered
  }

  /// Leaves the page on show for the next; answers whether it was the last, whose answers are now to be sent.
  mutating func next(_ questionnaire: PendingQuestionnaire) -> Bool {
    guard goesOn(questionnaire) else { return false }
    leave(questionnaire)
    if onLast(questionnaire) {
      return true
    }
    page += 1
    return false
  }

  /// Leaves the Question on show unanswered and goes on, as "Skip" does; answers whether it was the last.
  mutating func skip(_ questionnaire: PendingQuestionnaire) -> Bool {
    if pages.indices.contains(page) {
      pages[page] = Page()
    }
    return next(questionnaire)
  }

  /// Returns to the page before.
  mutating func back(_ questionnaire: PendingQuestionnaire) {
    guard page > 0 else { return }
    leave(questionnaire)
    page -= 1
  }

  /// What was given each Question, in their order, as the core takes them.
  var answers: [GivenAnswer] {
    pages.map { GivenAnswer(choices: $0.choices, text: $0.text) }
  }

  /// Seals a secret's answer as its page is left.
  private mutating func leave(_ questionnaire: PendingQuestionnaire) {
    guard questionnaire.questions.indices.contains(page), pages.indices.contains(page),
      questionnaire.questions[page].secret != nil,
      pages[page].answered
    else { return }
    pages[page].sealed = true
  }

  /// A sealed secret's answer is replaced whole by whatever is given it next.
  private mutating func unseal() {
    guard pages[page].sealed else { return }
    pages[page] = Page()
  }
}
