"use client";

import Link from "next/link";
import { useActionState } from "react";
import {
  chooseOrganization,
  createMyTeam,
  continueWithProvider,
  joinTeam,
  requestPasswordReset,
  resendVerification,
  resetPassword,
  signIn,
  signUp,
  startOver,
  verifyEmail,
  verifyTotp,
  type FormState,
} from "@/lib/auth/auth-actions";
import { MIN_PASSWORD } from "@/lib/auth/validate";
import { CodeField, Field, FormError, Notice, Submit } from "./fields";

const Hidden = ({ name, value }: { name: string; value?: string }) => (value ? <input type="hidden" name={name} value={value} /> : null);

// ---------------------------------------------------------------- Sign in

/**
 * Email first. An SSO domain goes straight to its identity provider (server
 * side); anyone else gets a password field, "Email me a code", and GitHub or
 * Google.
 */
export function SignInFlow({ returnTo, invitationToken, initial = {}, lockEmail }: { returnTo: string; invitationToken?: string; initial?: FormState; lockEmail?: boolean }) {
  const [state, action] = useActionState(signIn, initial);
  const step = state.step ?? "email";
  const email = state.email ?? "";
  const fieldError = (f: string) => (state.field === f ? state.error : undefined);
  const otherError = state.error && !["email", "password", "code"].includes(state.field ?? "") ? state.error : undefined;

  return (
    <>
      {state.notice ? <Notice>{state.notice}</Notice> : null}
      <FormError message={otherError} focus />
      <form action={action} className="auth-form" noValidate={false}>
        <Hidden name="returnTo" value={returnTo} />
        <Hidden name="invitationToken" value={invitationToken} />
        {step === "email" ? (
          <>
            <Field
              key="email"
              label="Email"
              name="email"
              type="email"
              autoComplete="email"
              inputMode="email"
              required
              defaultValue={email}
              placeholder="you@company.com"
              error={fieldError("email")}
              focus
            />
            <Submit name="intent" value="email" pending="Checking…">
              Continue
            </Submit>
          </>
        ) : (
          <>
            {/* The email stays in the form for the password manager and for each step. */}
            <div className="auth-email">
              <span className="faint">Signing in as</span> <strong>{email}</strong>
              {lockEmail ? null : (
                <Link href={`/sign-in${returnTo !== "/" ? `?returnTo=${encodeURIComponent(returnTo)}` : ""}`} className="link-muted">
                  Change
                </Link>
              )}
            </div>
            <input type="email" name="email" value={email} autoComplete="username" readOnly hidden />
            {step === "password" ? (
              <>
                <Field
                  key="password"
                  label="Password"
                  name="password"
                  type="password"
                  autoComplete="current-password"
                  error={fieldError("password")}
                  focus
                  end={
                    <Link href="/reset-password" className="link-muted" style={{ fontSize: 13 }}>
                      Forgot password?
                    </Link>
                  }
                />
                <Submit name="intent" value="password" pending="Signing in…">
                  Sign in
                </Submit>
                <Submit name="intent" value="send-code" secondary formNoValidate pending="Sending…">
                  Email me a code instead
                </Submit>
              </>
            ) : (
              <>
                <CodeField error={fieldError("code")} />
                <Submit name="intent" value="code" pending="Signing in…">
                  Sign in
                </Submit>
                <Submit name="intent" value="send-code" secondary formNoValidate pending="Sending…">
                  Send a new code
                </Submit>
              </>
            )}
          </>
        )}
      </form>
      <Providers returnTo={returnTo} invitationToken={invitationToken} />
    </>
  );
}

function GitHubMark() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}

function GoogleMark() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
      <path fill="#ededed" d="M21.6 12.23c0-.71-.06-1.4-.18-2.05H12v3.88h5.38a4.6 4.6 0 0 1-2 3.02v2.5h3.24c1.9-1.75 2.98-4.33 2.98-7.35Z" />
      <path fill="#bdbdbd" d="M12 22c2.7 0 4.96-.9 6.62-2.42l-3.24-2.5c-.9.6-2.04.95-3.38.95-2.6 0-4.8-1.75-5.59-4.11H3.07v2.58A10 10 0 0 0 12 22Z" />
      <path fill="#8f8f8f" d="M6.41 13.92A6 6 0 0 1 6.1 12c0-.67.11-1.31.31-1.92V7.5H3.07A10 10 0 0 0 2 12c0 1.61.39 3.14 1.07 4.5l3.34-2.58Z" />
      <path fill="#d4d4d4" d="M12 5.98c1.47 0 2.79.5 3.83 1.5l2.87-2.87C16.96 2.99 14.7 2 12 2a10 10 0 0 0-8.93 5.5l3.34 2.58C7.2 7.73 9.4 5.98 12 5.98Z" />
    </svg>
  );
}

/** "Continue with GitHub / Google": PKCE redirect, back through /callback. */
export function Providers({ returnTo, invitationToken }: { returnTo: string; invitationToken?: string }) {
  const [state, action] = useActionState(continueWithProvider, {});
  return (
    <>
      <div className="auth-or" role="separator">
        <span>or</span>
      </div>
      <form action={action} className="auth-form">
        <Hidden name="returnTo" value={returnTo} />
        <Hidden name="invitationToken" value={invitationToken} />
        <Submit name="provider" value="github" secondary pending="Opening GitHub…">
          <GitHubMark /> Continue with GitHub
        </Submit>
        <Submit name="provider" value="google" secondary pending="Opening Google…">
          <GoogleMark /> Continue with Google
        </Submit>
      </form>
      <FormError message={state.error} />
    </>
  );
}

// ---------------------------------------------------------------- Sign up

export function SignUpForm({ returnTo, invitationToken, email: fixedEmail, initial = {} }: { returnTo: string; invitationToken?: string; email?: string; initial?: FormState }) {
  const [state, action] = useActionState(signUp, initial);
  const v = state.values ?? {};
  const err = (f: string) => (state.field === f ? state.error : undefined);
  const otherError = state.error && !state.field ? state.error : undefined;
  // Re-mount the fields after each answer so they show what was sent back.
  const k = JSON.stringify(v) + (state.error ?? "");
  return (
    <>
      <FormError message={otherError} focus />
      <form action={action} className="auth-form" key={k}>
        <Hidden name="returnTo" value={returnTo} />
        <Hidden name="invitationToken" value={invitationToken} />
        <div className="auth-row">
          <Field label="First name" name="firstName" autoComplete="given-name" required defaultValue={v.firstName} error={err("firstName")} focus={!state.field || state.field === "firstName"} />
          <Field label="Last name" name="lastName" autoComplete="family-name" required defaultValue={v.lastName} error={err("lastName")} focus={state.field === "lastName"} />
        </div>
        {fixedEmail ? (
          <>
            <Field label="Email" type="email" value={fixedEmail} readOnly autoComplete="email" hint="From your invitation." />
            <input type="hidden" name="email" value={fixedEmail} />
          </>
        ) : (
          <Field label="Email" name="email" type="email" autoComplete="email" inputMode="email" required defaultValue={v.email} placeholder="you@company.com" error={err("email")} focus={state.field === "email"} />
        )}
        <Field
          label="Password"
          name="password"
          type="password"
          autoComplete="new-password"
          required
          minLength={MIN_PASSWORD}
          hint={`At least ${MIN_PASSWORD} characters. A short sentence works well.`}
          error={err("password")}
          focus={state.field === "password"}
        />
        <Submit pending="Creating your account…">Create account</Submit>
      </form>
      <Providers returnTo={returnTo} invitationToken={invitationToken} />
    </>
  );
}

// ---------------------------------------------------------------- Pending steps

export function VerifyEmailForm({ initial = {} }: { initial?: FormState }) {
  const [state, action] = useActionState(verifyEmail, initial);
  const [resent, resend, resending] = useActionState(resendVerification, {});
  if (state.expired || resent.expired) {
    // The step is gone (another browser, or it timed out): a code can't be used here any more, so offer the way back.
    return (
      <>
        <FormError message={state.expired ? state.error : resent.error} focus />
        <form action={startOver} className="auth-form">
          <button type="submit" className="btn solid lg auth-btn">
            Start again
          </button>
        </form>
      </>
    );
  }
  return (
    <>
      {resent.notice ? <Notice>{resent.notice}</Notice> : null}
      <FormError message={resent.error ?? (state.field ? undefined : state.error)} focus />
      <form action={action} className="auth-form">
        <CodeField error={state.field === "code" ? state.error : undefined} />
        <Submit pending="Checking…">Verify email</Submit>
      </form>
      <form action={resend} className="auth-form">
        <button type="submit" className="btn lg auth-btn" disabled={resending}>
          {resending ? "Sending…" : "Send a new code"}
        </button>
      </form>
    </>
  );
}

/** "Join <team>": a signed-in person accepts an invitation addressed to their own email. */
export function JoinTeamForm({ invitationId, teamName, secondary }: { invitationId: string; teamName: string; secondary?: boolean }) {
  const [state, action] = useActionState(joinTeam, {});
  return (
    <>
      <FormError message={state.error} focus />
      <form action={action} className="auth-form">
        <input type="hidden" name="invitationId" value={invitationId} />
        <Submit secondary={secondary} pending="Joining…">
          Join {teamName}
        </Submit>
      </form>
    </>
  );
}

export function CreateTeamForm({ secondary }: { secondary?: boolean }) {
  const [state, action] = useActionState(createMyTeam, {});
  return (
    <>
      <FormError message={state.error} focus />
      <form action={action} className="auth-form">
        <Submit secondary={secondary} pending="Creating your team…">
          Create my team
        </Submit>
      </form>
    </>
  );
}

export function OrganizationPicker({ organizations, initial = {} }: { organizations: { id: string; name: string }[]; initial?: FormState }) {
  const [state, action] = useActionState(chooseOrganization, initial);
  return (
    <>
      <FormError message={state.error} focus />
      <form action={action} className="auth-form">
        <fieldset className="auth-orgs">
          <legend className="sr-only">Organizations</legend>
          {organizations.map((o, i) => (
            <label key={o.id} className="auth-org">
              <input type="radio" name="organizationId" value={o.id} defaultChecked={i === 0} required />
              <span className="team-mark" aria-hidden="true" />
              <span>{o.name}</span>
            </label>
          ))}
        </fieldset>
        <Submit pending="Signing in…">Continue</Submit>
      </form>
    </>
  );
}

export function TotpForm({ initial = {} }: { initial?: FormState }) {
  const [state, action] = useActionState(verifyTotp, initial);
  return (
    <>
      <FormError message={state.field ? undefined : state.error} focus />
      <form action={action} className="auth-form">
        <CodeField label="Code from your authenticator app" error={state.field === "code" ? state.error : undefined} />
        <Submit pending="Checking…">Verify</Submit>
      </form>
    </>
  );
}

export function StartOver() {
  return (
    <form action={startOver}>
      <button type="submit" className="link-muted auth-link-btn">
        Start over
      </button>
    </form>
  );
}

// ---------------------------------------------------------------- Password reset

export function ResetRequestForm({ initial = {} }: { initial?: FormState }) {
  const [state, action] = useActionState(requestPasswordReset, initial);
  if (state.notice) return <Notice>{state.notice}</Notice>;
  return (
    <>
      <FormError message={state.field ? undefined : state.error} focus />
      <form action={action} className="auth-form">
        <Field label="Email" name="email" type="email" autoComplete="email" inputMode="email" required placeholder="you@company.com" error={state.field === "email" ? state.error : undefined} focus />
        <Submit pending="Sending…">Email me a reset link</Submit>
      </form>
    </>
  );
}

export function NewPasswordForm({ token, initial = {} }: { token: string; initial?: FormState }) {
  const [state, action] = useActionState(resetPassword, initial);
  return (
    <>
      <FormError message={state.field ? undefined : state.error} focus />
      <form action={action} className="auth-form">
        <input type="hidden" name="token" value={token} />
        <Field
          label="New password"
          name="password"
          type="password"
          autoComplete="new-password"
          required
          minLength={MIN_PASSWORD}
          hint={`At least ${MIN_PASSWORD} characters.`}
          error={state.field === "password" ? state.error : undefined}
          focus
        />
        <Submit pending="Saving…">Set password and sign in</Submit>
      </form>
    </>
  );
}
