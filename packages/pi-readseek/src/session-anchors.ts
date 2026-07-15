/**
 * Tracks files that have fresh hashline anchors in the current extension session.
 */
export class SessionAnchors {
  readonly #paths = new Set<string>();

  /**
   * Records that a file has produced anchors usable by later edit calls.
   */
  markAnchored(absolutePath: string): void {
    this.#paths.add(absolutePath);
  }

  /**
   * Forgets anchors after a file mutation that did not return current anchors.
   */
  forget(absolutePath: string): void {
    this.#paths.delete(absolutePath);
  }

  /**
   * Forgets every anchor, such as when resetting extension session state.
   */
  clear(): void {
    this.#paths.clear();
  }

  /**
   * Returns whether a file has fresh anchors in the current session.
   */
  hasFreshAnchors(absolutePath: string): boolean {
    return this.#paths.has(absolutePath);
  }
}
