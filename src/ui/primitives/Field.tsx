import { useId } from "react";
import type { InputHTMLAttributes, ReactNode, SelectHTMLAttributes } from "react";
import { cx } from "../cx";
import { Label } from "./Text";
import "./Field.css";

/// Wraps any control with a label and a hint, and wires the label to it. Use
/// this directly only for controls the library does not provide; `TextInput`,
/// `Select`, and `Switch` build it in.
export interface FieldProps {
  label?: ReactNode;
  hint?: ReactNode;
  /// Shown in place of the hint, in the fault colour.
  error?: ReactNode;
  /// Label sits beside the control rather than above it.
  inline?: boolean;
  htmlFor?: string;
  /// Id stamped on the hint/error paragraph. Pass the control's
  /// `aria-describedby` the same value so the message is actually announced.
  /// `TextInput`, `Select`, and `Switch` wire this for you.
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
}

/// The bare control, with no label or hint around it. Reach for this when the
/// input shares a row with something else (a Save button, a unit suffix) and
/// `TextInput`'s built-in `Field` would force it onto its own line.
export function Input({ invalid = false, className, ...rest }: InputProps) {
  return (
    <input
      aria-invalid={invalid || undefined}
      className={cx("ui-input", invalid && "ui-input--invalid", className)}
      {...rest}
    />
  );
}

export interface TextInputProps extends InputProps {
  label?: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  fieldClassName?: string;
}

/// `Field` + `Input`, which is the shape most settings want.
export function TextInput({
  label,
  hint,
  error,
  invalid = false,
  fieldClassName,
  id,
  ...rest
}: TextInputProps) {
  const generated = useId();
  const inputId = id ?? generated;
  const msgId = `${inputId}-msg`;
  const hasMessage = Boolean(error ?? hint);
  return (
    <Field
      label={label}
      hint={hint}
      error={error}
      htmlFor={inputId}
      messageId={hasMessage ? msgId : undefined}
      className={fieldClassName}
    >
      <Input
        id={inputId}
        invalid={invalid || Boolean(error)}
        aria-describedby={hasMessage ? msgId : undefined}
        {...rest}
      />
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
  fieldClassName?: string;
}

/// A native `<select>`, restyled in its closed state only. The open menu stays
/// the real macOS popup, which keeps type-ahead, keyboard navigation, and
/// VoiceOver working without reimplementing any of it.
export function Select({
  label,
  hint,
  error,
  options,
  fieldClassName,
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
      className={fieldClassName}
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
  fieldClassName?: string;
}

/// A real checkbox behind a drawn track, so keyboard and assistive-technology
/// behaviour is the platform's rather than ours.
export function Switch({ label, hint, fieldClassName, className, id, ...rest }: SwitchProps) {
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
      className={fieldClassName}
    >
      {control}
    </Field>
  );
}
