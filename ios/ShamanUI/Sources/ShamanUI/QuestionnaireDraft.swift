import ShamanCore

/// What a reader has half given a Questionnaire, one Question to a page: the page they are on and, for each Question,
/// the choices taken and what was written. It is the screen's own, kept while the Session's screen stays open, so a
/// Questionnaire swiped away and reviewed returns as it was left; the core is handed the answers whole when they are
/// sent. A secret's answer is never shown again once the reader has left it: its page, or the sheet.
struct QuestionnaireDraft: Equatable, Sendable {
  /// What was given one Question.
  struct Page: Equatable, Sendable {
    /// Whether it is a secret Question's, whose choices taken are never marked and whose answer, once left, shows
    /// nothing of what it was.
    var secret = false
    var choices: [String] = []
    var text = ""
    /// Whether a secret's answer was left behind, its page or the sheet left since it was given, so it reads
    /// "Answered (secret)" and shows nothing of what it was.
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
    pages = questionnaire.questions.map { Page(secret: $0.secret != nil) }
  }

  /// Whether the page on show is the last, whose tap sends the answers.
  func onLast(_ questionnaire: PendingQuestionnaire) -> Bool {
    page >= questionnaire.questions.count - 1
  }

  /// Whether the page on show holds a secret's answer left behind, which shows nothing of what it was.
  var sealed: Bool { pages.indices.contains(page) && pages[page].sealed }

  /// Whether the page on show reads "Answered (secret)": a secret's answer left behind, or a choice taken of a
  /// secret's, which no card marks.
  var readsSecret: Bool {
    guard pages.indices.contains(page), pages[page].secret else { return false }
    return pages[page].sealed || !pages[page].choices.isEmpty
  }

  /// Whether `choice` shows as taken on the page on show: never a secret's, as the TUI marks none.
  func taken(_ choice: String) -> Bool {
    guard pages.indices.contains(page), !pages[page].secret else { return false }
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
    setAside()
    if onLast(questionnaire) {
      return true
    }
    page += 1
    return false
  }

  /// Leaves the Question on show unanswered, as "Skip" does, and goes on to the next; on the last page it stays,
  /// for "Send answers" alone sends.
  mutating func skip(_ questionnaire: PendingQuestionnaire) {
    guard pages.indices.contains(page) else { return }
    pages[page] = Page(secret: pages[page].secret)
    if !onLast(questionnaire) {
      page += 1
    }
  }

  /// Returns to the page before.
  mutating func back() {
    guard page > 0 else { return }
    setAside()
    page -= 1
  }

  /// What was given each Question, in their order, as the core takes them.
  var answers: [GivenAnswer] {
    pages.map { GivenAnswer(choices: $0.choices, text: $0.text) }
  }

  /// Seals a secret's answer on the page on show, as its page is left or the sheet swiped away.
  mutating func setAside() {
    guard pages.indices.contains(page), pages[page].secret, pages[page].answered else { return }
    pages[page].sealed = true
  }

  /// A sealed secret's answer is replaced whole by whatever is given it next.
  private mutating func unseal() {
    guard pages[page].sealed else { return }
    pages[page] = Page(secret: true)
  }
}
