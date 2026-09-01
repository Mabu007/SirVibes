import { useState } from "react";
import { Avatar, Button } from "@heroui/react";
import logoUrl from "../assets/logo.png";
import { explainAuthError, resetPassword, signIn, signUp } from "../lib/firebase";

/**
 * The door. Deliberately plain: an email, a password, and nothing else to
 * decide before the work starts.
 */
export function LoginScreen() {
  const [mode, setMode] = useState<"in" | "up">("in");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      if (mode === "up") await signUp(email.trim(), password);
      else await signIn(email.trim(), password);
      // The auth listener in App takes it from here.
    } catch (err) {
      setError(explainAuthError(err));
    } finally {
      setBusy(false);
    }
  };

  const forgot = async () => {
    if (!email.trim()) {
      setError("Enter your email first and I will send a reset link.");
      return;
    }
    try {
      await resetPassword(email.trim());
      setNote("Sent. Check your email for a link to set a new password.");
      setError(null);
    } catch (err) {
      setError(explainAuthError(err));
    }
  };

  return (
    <div className="grid h-full place-items-center bg-background-secondary px-6">
      <div className="w-full max-w-sm">
        <div className="mb-7 flex flex-col items-center gap-3">
          <Avatar size="lg" className="bg-[#141210]">
            <Avatar.Image src={logoUrl} alt="SirVibe" className="scale-[0.72]" />
            <Avatar.Fallback>SV</Avatar.Fallback>
          </Avatar>
          <div className="text-center">
            <h1 className="text-lg font-semibold text-foreground">SirVibe</h1>
            <p className="mt-1 text-[13px] text-muted">
              {mode === "in" ? "Sign in to pick up where you left off." : "Create an account to get started."}
            </p>
          </div>
        </div>

        <form onSubmit={submit} className="flex flex-col gap-2.5">
          <input
            type="email"
            autoFocus
            autoComplete="email"
            placeholder="you@company.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="rounded-xl border border-field-border bg-field px-3.5 py-2.5 text-sm outline-none focus:border-accent"
          />
          <input
            type="password"
            autoComplete={mode === "in" ? "current-password" : "new-password"}
            placeholder="Password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className="rounded-xl border border-field-border bg-field px-3.5 py-2.5 text-sm outline-none focus:border-accent"
          />

          {error && (
            <div className="rounded-lg bg-danger/[0.08] px-3 py-2 text-[12.5px] text-danger">
              {error}
            </div>
          )}
          {note && (
            <div className="rounded-lg bg-success/[0.08] px-3 py-2 text-[12.5px] text-success">
              {note}
            </div>
          )}

          <Button
            type="submit"
            variant="primary"
            isDisabled={busy || !email.trim() || !password}
            className="mt-1 w-full"
          >
            {busy ? "One moment…" : mode === "in" ? "Sign in" : "Create account"}
          </Button>
        </form>

        <div className="mt-4 flex items-center justify-between text-[12.5px]">
          <button
            onClick={() => {
              setMode(mode === "in" ? "up" : "in");
              setError(null);
              setNote(null);
            }}
            className="text-accent hover:underline"
          >
            {mode === "in" ? "Create an account" : "I already have an account"}
          </button>
          {mode === "in" && (
            <button onClick={forgot} className="text-muted hover:text-foreground">
              Forgot password
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
