import { useId } from "react";
import type { InputHTMLAttributes, ReactNode, SelectHTMLAttributes } from "react";
import { cx } from "../cx";
import { Label } from "./Text";
import "./Field.css";

/// A label and hint around any control. `Input`, `Select` and `Switch` build
/// it in.
export interface FieldProps {
  label?: ReactNode;
  hint?: ReactNode;
  /// Shown in place of the hint, in the fault colour.
  error?: ReactNode;
  /// Label sits beside the control rather than above it.
  inline?: boolean;
  htmlFor?: string;
  /// Give the control the same `aria-describedby`, or nothing is announced.
  messageId?: string;
  className?: string;
  children: ReactNode;
}

export function Field({
  label,
  hint,
  error,
  inline = false,
  htmlFor,
  messageId,
  className,
  children,
}: FieldProps) {
  const message = error ?? hint;
  return (
    <div className={cx("ui-field", inline && "ui-field--inline", className)}>
      {label && (
        <Label as="label" htmlFor={htmlFor} tone={inline ? "strong" : "default"}>
          {label}
        </Label>
      )}
      {children}
      {message && (
        <p id={messageId} className={cx("ui-field__hint", error && "ui-field__hint--fault")}>
          {message}
        </p>
      )}
    </div>
  );
}

export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  invalid?: boolean;
  label?: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
}

/// Bare when it shares a row with another control, wrapped in `Field` the
/// moment a label, hint or error is given.
export function Input({
  invalid = false,
  label,
  hint,
  error,
  className,
  id,
  ...rest
}: InputProps) {
  const generated = useId();
  const inputId = id ?? generated;
  const msgId = `${inputId}-msg`;
  const hasMessage = Boolean(error ?? hint);
  const control = (
    <input
      id={inputId}
      aria-invalid={invalid || Boolean(error) || undefined}
      aria-describedby={hasMessage ? msgId : undefined}
      className={cx("ui-input", (invalid || Boolean(error)) && "ui-input--invalid", className)}
      {...rest}
    />
  );

  if (!label && !hasMessage) return control;

  return (
    <Field
      label={label}
      hint={hint}
      error={error}
      htmlFor={inputId}
      messageId={hasMessage ? msgId : undefined}
    >
      {control}
    </Field>
  );
}

export interface SelectOption {
  value: string;
  label: string;
}

export interface SelectProps extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "children"> {
  label?: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  options: SelectOption[];
}

/// Restyled closed only: the open menu stays the real macOS popup, with its
/// type-ahead and VoiceOver intact.
export function Select({
  label,
  hint,
  error,
  options,
  className,
  id,
  ...rest
}: SelectProps) {
  const generated = useId();
  const selectId = id ?? generated;
  const msgId = `${selectId}-msg`;
  const hasMessage = Boolean(error ?? hint);
  return (
    <Field
      label={label}
      hint={hint}
      error={error}
      htmlFor={selectId}
      messageId={hasMessage ? msgId : undefined}
    >
      <select
        id={selectId}
        aria-describedby={hasMessage ? msgId : undefined}
        className={cx("ui-select", className)}
        {...rest}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </Field>
  );
}

export interface SwitchProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "type" | "size"> {
  label?: ReactNode;
  hint?: ReactNode;
}

/// A real checkbox behind a drawn track, so the behaviour is the platform's.
export function Switch({ label, hint, className, id, ...rest }: SwitchProps) {
  const generated = useId();
  const inputId = id ?? generated;
  const msgId = `${inputId}-msg`;
  const control = (
    <span className={cx("ui-switch", className)}>
      <input
        id={inputId}
        type="checkbox"
        className="ui-switch__input"
        aria-describedby={hint ? msgId : undefined}
        {...rest}
      />
      <span className="ui-switch__track" />
      <span className="ui-switch__thumb" />
    </span>
  );

  if (!label) return control;

  return (
    <Field
      label={label}
      hint={hint}
      htmlFor={inputId}
      messageId={hint ? msgId : undefined}
      inline
    >
      {control}
    </Field>
  );
}
