"use client";

import { useEffect, useId, useRef, type InputHTMLAttributes, type ReactNode } from "react";
import { useFormStatus } from "react-dom";

/** A labelled input. Shows its error under it, marks itself invalid and takes focus when the error is about it. */
export function Field({
  label,
  error,
  hint,
  focus,
  end,
  ...input
}: InputHTMLAttributes<HTMLInputElement> & { label: string; error?: string; hint?: ReactNode; focus?: boolean; end?: ReactNode }) {
  const id = useId();
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (focus) {
      ref.current?.focus();
      ref.current?.select();
    }
  }, [focus, error]);
  const describedBy = [error ? `${id}-err` : null, hint ? `${id}-hint` : null].filter(Boolean).join(" ") || undefined;
  return (
    <div className="auth-field">
      <div className="row-between" style={{ alignItems: "baseline" }}>
        <label htmlFor={id}>{label}</label>
        {end}
      </div>
      <input ref={ref} id={id} className="input auth-input" aria-invalid={error ? true : undefined} aria-describedby={describedBy} {...input} />
      {hint ? (
        <span id={`${id}-hint`} className="auth-hint">
          {hint}
        </span>
      ) : null}
      {error ? (
        <span id={`${id}-err`} className="auth-error" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  );
}

/** A six-digit code: one input, numeric keyboard, filled by SMS/email autofill, and a pasted "123 456" or "123-456" becomes 123456. */
export function CodeField({ label = "Code", error, focus = true, name = "code" }: { label?: string; error?: string; focus?: boolean; name?: string }) {
  return (
    <Field
      label={label}
      name={name}
      error={error}
      focus={focus}
      inputMode="numeric"
      autoComplete="one-time-code"
      maxLength={6}
      required
      placeholder="123456"
      className="input auth-input auth-code"
      onPaste={(e) => {
        const digits = e.clipboardData.getData("text").replace(/\D/g, "").slice(0, 6);
        if (digits) {
          e.preventDefault();
          e.currentTarget.value = digits;
          if (digits.length === 6) e.currentTarget.form?.requestSubmit();
        }
      }}
      onInput={(e) => {
        const el = e.currentTarget;
        const digits = el.value.replace(/\D/g, "").slice(0, 6);
        if (digits !== el.value) el.value = digits;
      }}
    />
  );
}

/** The form's submit button: disabled with a pending label while the action runs. */
export function Submit({ children, pending: label, secondary, ...rest }: { children: ReactNode; pending?: string; secondary?: boolean; name?: string; value?: string; formNoValidate?: boolean }) {
  const { pending } = useFormStatus();
  return (
    <button type="submit" className={`btn lg auth-btn ${secondary ? "" : "solid"}`} disabled={pending} aria-disabled={pending} {...rest}>
      {pending ? (label ?? "Working…") : children}
    </button>
  );
}

/** An error that is not about one field. Announced, and focusable so focus can land on it. */
export function FormError({ message, focus }: { message?: string; focus?: boolean }) {
  const ref = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    if (focus && message) ref.current?.focus();
  }, [focus, message]);
  if (!message) return null;
  return (
    <p ref={ref} tabIndex={-1} role="alert" className="toast err">
      {message}
    </p>
  );
}

export function Notice({ children }: { children: ReactNode }) {
  return (
    <p role="status" className="toast ok">
      {children}
    </p>
  );
}
