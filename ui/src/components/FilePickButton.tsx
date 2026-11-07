import { Button } from "@vitavision/ui";
import { useRef, type ReactNode } from "react";

/** A button that opens the file chooser and hands over one file. */
export function FilePickButton({
  accept,
  icon,
  children,
  onFile,
  disabled,
  variant = "secondary",
}: {
  accept: string;
  icon?: ReactNode;
  children: ReactNode;
  onFile: (file: File) => void;
  disabled?: boolean;
  variant?: "primary" | "secondary" | "ghost";
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <Button
        variant={variant}
        icon={icon}
        disabled={disabled}
        onClick={() => input.current?.click()}
      >
        {children}
      </Button>
      <input
        ref={input}
        type="file"
        accept={accept}
        className="hidden"
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = "";
          if (f) onFile(f);
        }}
      />
    </>
  );
}
