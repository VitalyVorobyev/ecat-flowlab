/** Offer `data` as a JSON file download. */
export function downloadJson(name: string, data: unknown): void {
  downloadText(name, JSON.stringify(data, null, 2), "application/json");
}

/** Offer `text` as a file download with the given MIME type. */
export function downloadText(name: string, text: string, type = "text/plain"): void {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}
