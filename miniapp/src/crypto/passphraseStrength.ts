/**
 * A minimum-entropy floor on the vault passphrase. Argon2id (crypto/kdf.ts) raises the cost of
 * brute-forcing a stolen ciphertext, but that only buys time against a passphrase that has real
 * entropy to begin with -- KDF cost cannot rescue "password1".
 *
 * Deliberately not zxcvbn: its dictionary-based scoring is meaningfully better, but ships
 * several hundred KB of wordlists into a bundle whose entire justification for staying small is
 * that every dependency in it can read the user's key (see the README's dependency-by-dependency
 * accounting) -- for one screen's input validation, that trade is not worth it. This is a coarse
 * length/charset entropy estimate plus a short deny-list of the most common weak passphrases,
 * not a full strength model. It will pass some guessable passphrases a proper model would catch,
 * and there is no pretending otherwise -- see the caveat on the exported constant below.
 */

const COMMON_WEAK_PASSPHRASES = new Set([
  "password",
  "password1",
  "passphrase",
  "123456",
  "12345678",
  "123456789",
  "qwerty",
  "qwertyuiop",
  "letmein",
  "changeme",
  "solana",
  "wallet123",
  "iloveyou",
  "welcome",
  "admin",
  "abc123",
]);

export interface PassphraseStrength {
  bitsEstimate: number;
  /** 0 (unacceptable) through 4 (strong). */
  score: 0 | 1 | 2 | 3 | 4;
  feedback: string[];
}

/**
 * The score below which the UI should refuse to proceed. Chosen so a short all-lowercase
 * dictionary word or an 8-digit PIN both land under it, while a random ~10+ character passphrase
 * or a multi-word passphrase clears it -- calibrated by feel, not measured against a corpus,
 * which is exactly the kind of gap a proper zxcvbn-style model closes and this estimate does not.
 */
export const MIN_ACCEPTABLE_SCORE = 2;

function charsetSize(passphrase: string): number {
  let size = 0;
  if (/[a-z]/.test(passphrase)) size += 26;
  if (/[A-Z]/.test(passphrase)) size += 26;
  if (/[0-9]/.test(passphrase)) size += 10;
  if (/[^a-zA-Z0-9]/.test(passphrase)) size += 32;
  return Math.max(size, 1);
}

function hasLongRepeatedRun(passphrase: string): boolean {
  return /(.)\1{3,}/.test(passphrase); // same character 4+ times in a row
}

export function estimatePassphraseStrength(passphrase: string): PassphraseStrength {
  const feedback: string[] = [];
  const normalized = passphrase.trim();

  if (COMMON_WEAK_PASSPHRASES.has(normalized.toLowerCase())) {
    return {
      bitsEstimate: 0,
      score: 0,
      feedback: ["This is one of the most commonly used passphrases -- choose something unique."],
    };
  }

  const bitsEstimate = normalized.length * Math.log2(charsetSize(normalized));

  if (normalized.length < 8) {
    feedback.push("Use at least 8 characters -- longer is better than adding symbols.");
  }
  if (hasLongRepeatedRun(normalized)) {
    feedback.push("Avoid long runs of the same character.");
  }

  let score: PassphraseStrength["score"];
  if (normalized.length < 8 || bitsEstimate < 28) {
    score = 0;
  } else if (bitsEstimate < 36) {
    score = 1;
  } else if (bitsEstimate < 60) {
    score = 2;
  } else if (bitsEstimate < 80) {
    score = 3;
  } else {
    score = 4;
  }

  if (score < MIN_ACCEPTABLE_SCORE && feedback.length === 0) {
    feedback.push("Use a longer passphrase, or a mix of a few unrelated words.");
  }

  return { bitsEstimate, score, feedback };
}
