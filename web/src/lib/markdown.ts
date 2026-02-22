import { Renderer, marked } from "marked";

const renderer = new Renderer();
const defaultLinkRenderer = renderer.link.bind(renderer);
renderer.link = function (token) {
  const html = defaultLinkRenderer(token);
  return html.replace("<a ", '<a target="_blank" rel="noopener" ');
};
marked.setOptions({ renderer, gfm: true, breaks: true });

export function renderMarkdown(text: string): string {
  return marked.parse(text, { async: false }) as string;
}
