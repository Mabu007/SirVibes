/**
 * The security rules, executed.
 *
 * These run against the Firestore emulator loading `firebase/firestore.rules`
 * — the same file that gets deployed — so a pass here means the rules really
 * behave this way, not that they look right.
 *
 *   npx firebase emulators:exec --only firestore "node firebase/test/rules.test.mjs"
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  assertFails,
  assertSucceeds,
  initializeTestEnvironment,
} from "@firebase/rules-unit-testing";
import { doc, getDoc, setDoc, deleteDoc, collection, getDocs } from "firebase/firestore";

const env = await initializeTestEnvironment({
  projectId: "sirvibes-44204",
  firestore: {
    rules: readFileSync("firebase/firestore.rules", "utf8"),
    host: "127.0.0.1",
    port: 8080,
  },
});

const results = [];
const check = async (name, fn) => {
  try {
    await fn();
    results.push(`  PASS  ${name}`);
  } catch (error) {
    results.push(`  FAIL  ${name}\n        ${error.message}`);
    process.exitCode = 1;
  }
};

const alice = env.authenticatedContext("alice-uid").firestore();
const bob = env.authenticatedContext("bob-uid").firestore();
const stranger = env.unauthenticatedContext().firestore();

const chatOf = (db, uid, id = "chat-1") => doc(db, "users", uid, "chats", id);
const messageOf = (db, uid, id = "chat-1", m = "0001") =>
  doc(db, "users", uid, "chats", id, "messages", m);

// ---- a signed-in person owns their own corner ------------------------------

await check("owner creates their own chat", () =>
  assertSucceeds(
    setDoc(chatOf(alice, "alice-uid"), { title: "Podcast Short Edit", status: "completed" }),
  ),
);

await check("owner reads their own chat", () =>
  assertSucceeds(getDoc(chatOf(alice, "alice-uid"))),
);

await check("owner updates their own chat (rename)", () =>
  assertSucceeds(setDoc(chatOf(alice, "alice-uid"), { title: "Podcast Shorts — September" }, { merge: true })),
);

await check("owner lists their own chats", () =>
  assertSucceeds(getDocs(collection(alice, "users", "alice-uid", "chats"))),
);

await check("owner writes a message under their own chat", () =>
  assertSucceeds(setDoc(messageOf(alice, "alice-uid"), { role: "user", content: "hello" })),
);

await check("owner deletes their own chat", () =>
  assertSucceeds(deleteDoc(chatOf(alice, "alice-uid", "throwaway"))),
);

// ---- and nobody else's ------------------------------------------------------

await check("another user cannot read it", () =>
  assertFails(getDoc(chatOf(bob, "alice-uid"))),
);

await check("another user cannot overwrite it", () =>
  assertFails(setDoc(chatOf(bob, "alice-uid"), { title: "hijacked" })),
);

await check("another user cannot delete it", () =>
  assertFails(deleteDoc(chatOf(bob, "alice-uid"))),
);

await check("another user cannot list someone else's chats", () =>
  assertFails(getDocs(collection(bob, "users", "alice-uid", "chats"))),
);

await check("another user cannot read someone else's messages", () =>
  assertFails(getDoc(messageOf(bob, "alice-uid"))),
);

await check("another user cannot reach the parent user document", () =>
  assertFails(getDoc(doc(bob, "users", "alice-uid"))),
);

// ---- signed out means out ---------------------------------------------------

await check("signed out cannot read a chat", () =>
  assertFails(getDoc(chatOf(stranger, "alice-uid"))),
);

await check("signed out cannot write a chat", () =>
  assertFails(setDoc(chatOf(stranger, "alice-uid"), { title: "anonymous" })),
);

await check("signed out cannot write even under an unused uid", () =>
  assertFails(setDoc(chatOf(stranger, "nobody-uid"), { title: "squatting" })),
);

// ---- nothing outside a user's tree is reachable at all ----------------------

await check("no client may write outside users/{uid}", () =>
  assertFails(setDoc(doc(alice, "chats", "loose"), { title: "stray" })),
);

await check("no client may read a collection at the root", () =>
  assertFails(getDocs(collection(alice, "chats"))),
);

// A uid that merely looks like a prefix of another must not match it.
await check("a lookalike uid gets nothing", () =>
  assertFails(getDoc(doc(env.authenticatedContext("alice").firestore(), "users", "alice-uid", "chats", "chat-1"))),
);

console.log("\nFirestore rules:\n" + results.join("\n") + "\n");
console.log(
  `  ${results.filter((r) => r.startsWith("  PASS")).length} passed, ` +
    `${results.filter((r) => r.startsWith("  FAIL")).length} failed\n`,
);
await env.cleanup();
