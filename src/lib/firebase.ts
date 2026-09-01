import { initializeApp } from "firebase/app";
import {
  browserLocalPersistence,
  createUserWithEmailAndPassword,
  onAuthStateChanged,
  sendPasswordResetEmail,
  setPersistence,
  signInWithEmailAndPassword,
  signOut,
  type User,
} from "firebase/auth";
import { getAuth } from "firebase/auth";
import {
  collection,
  deleteDoc,
  doc,
  getDocs,
  getFirestore,
  limit,
  orderBy,
  query,
  setDoc,
  Timestamp,
} from "firebase/firestore";

/**
 * Identity and the small, portable part of a person's work.
 *
 * This is the client configuration: it identifies the project and is meant to
 * ship. It is not a credential — every read and write is checked against
 * Firestore's rules using the signed-in user's own token, so the config on its
 * own grants nothing. No admin key belongs anywhere near this file.
 */
const firebaseConfig = {
  apiKey: "AIzaSyCy-6HhG5cih6csrVZaNQ5ul7F_tC-i3k0",
  authDomain: "sirvibes-44204.firebaseapp.com",
  projectId: "sirvibes-44204",
  storageBucket: "sirvibes-44204.firebasestorage.app",
  messagingSenderId: "778526817917",
  appId: "1:778526817917:web:46e94afeef939180903b14",
  measurementId: "G-JBRHZ5YDKF",
};

const app = initializeApp(firebaseConfig);
export const auth = getAuth(app);
const db = getFirestore(app);

/** Stay signed in across restarts; a desktop app should not ask every launch. */
void setPersistence(auth, browserLocalPersistence);

export type { User };

export const watchUser = (fn: (user: User | null) => void) => onAuthStateChanged(auth, fn);

export const signUp = (email: string, password: string) =>
  createUserWithEmailAndPassword(auth, email, password);

export const signIn = (email: string, password: string) =>
  signInWithEmailAndPassword(auth, email, password);

export const signOutUser = () => signOut(auth);

export const resetPassword = (email: string) => sendPasswordResetEmail(auth, email);

/** Firebase's messages are written for developers; these are for people. */
export function explainAuthError(error: unknown): string {
  const code = (error as { code?: string })?.code ?? "";
  switch (code) {
    case "auth/invalid-email":
      return "That does not look like an email address.";
    case "auth/missing-password":
    case "auth/weak-password":
      return "Passwords need to be at least six characters.";
    case "auth/email-already-in-use":
      return "There is already an account with that email. Try signing in.";
    case "auth/invalid-credential":
    case "auth/wrong-password":
    case "auth/user-not-found":
      return "That email and password do not match an account.";
    case "auth/too-many-requests":
      return "Too many attempts. Wait a moment and try again.";
    case "auth/network-request-failed":
      return "Could not reach Firebase. Check the connection and try again.";
    default:
      return error instanceof Error ? error.message : "Could not sign in.";
  }
}

// ------------------------------------------------------------------ chats

/**
 * What travels with the account: the shape of the work, not the work itself.
 * Videos, renders and source media stay on the machine that made them — this
 * is what lets someone sit down at another install and find their chats.
 */
export interface CloudChat {
  id: string;
  title: string;
  status: string;
  workspace: string | null;
  updatedMs: number;
}

const chatsRef = (uid: string) => collection(db, "users", uid, "chats");

export async function loadChats(uid: string, max = 50): Promise<CloudChat[]> {
  const snapshot = await getDocs(
    query(chatsRef(uid), orderBy("updatedAt", "desc"), limit(max)),
  );
  return snapshot.docs.map((d) => {
    const data = d.data();
    return {
      id: d.id,
      title: (data.title as string) ?? "Untitled",
      status: (data.status as string) ?? "idle",
      workspace: (data.workspace as string | null) ?? null,
      updatedMs: (data.updatedAt as Timestamp | undefined)?.toMillis?.() ?? 0,
    };
  });
}

export async function saveChat(uid: string, chat: CloudChat): Promise<void> {
  await setDoc(
    doc(chatsRef(uid), chat.id),
    {
      title: chat.title,
      status: chat.status,
      workspace: chat.workspace,
      updatedAt: Timestamp.fromMillis(chat.updatedMs || Date.now()),
    },
    { merge: true },
  );
}

export async function deleteChat(uid: string, chatId: string): Promise<void> {
  await deleteDoc(doc(chatsRef(uid), chatId));
}

/**
 * The transcript, in the account. Deliberately capped: a chat that ran for an
 * hour of tool calls belongs on disk, and what is worth carrying between
 * machines is the conversation, not every line of a render log.
 */
export async function saveMessages(
  uid: string,
  chatId: string,
  messages: { role: string; content: string }[],
  max = 200,
): Promise<void> {
  const recent = messages.slice(-max);
  await Promise.all(
    recent.map((message, index) =>
      setDoc(doc(collection(chatsRef(uid), chatId, "messages"), String(index).padStart(4, "0")), {
        role: message.role,
        content: (message.content ?? "").slice(0, 8000),
        index,
      }),
    ),
  );
}
