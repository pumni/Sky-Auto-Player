export interface RenderedTextContrast {
  text: string;
  element: string;
  foreground: string;
  renderedForeground: string;
  background: string;
  ratio: number;
  minimum: number;
  exemption: string | null;
}

export function collectRenderedTextContrast(): RenderedTextContrast[] {
  type Color = { r: number; g: number; b: number; a: number };

  const parseColor = (value: string): Color => {
    if (value === 'transparent') return { r: 0, g: 0, b: 0, a: 0 };
    const channels = value.match(/[\d.]+/g)?.map(Number) ?? [];
    if (channels.length < 3) throw new Error(`Cannot parse computed color: ${value}`);
    return {
      r: channels[0],
      g: channels[1],
      b: channels[2],
      a: channels[3] ?? 1,
    };
  };

  const composite = (front: Color, back: Color): Color => {
    const alpha = front.a + back.a * (1 - front.a);
    if (alpha <= 0) return { r: 255, g: 255, b: 255, a: 1 };
    return {
      r: (front.r * front.a + back.r * back.a * (1 - front.a)) / alpha,
      g: (front.g * front.a + back.g * back.a * (1 - front.a)) / alpha,
      b: (front.b * front.a + back.b * back.a * (1 - front.a)) / alpha,
      a: alpha,
    };
  };

  const cssColor = (color: Color) =>
    `rgb(${Math.round(color.r)}, ${Math.round(color.g)}, ${Math.round(color.b)})`;

  const luminance = (color: Color) => {
    const linear = [color.r, color.g, color.b].map((channel) => {
      const value = channel / 255;
      return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    });
    return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
  };

  const ratio = (first: Color, second: Color) => {
    const a = luminance(first);
    const b = luminance(second);
    return Number(((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)).toFixed(2));
  };

  const renderedBackground = (element: Element) => {
    const ancestors: Element[] = [];
    for (let node: Element | null = element; node; node = node.parentElement) {
      ancestors.push(node);
    }

    let background: Color = { r: 255, g: 255, b: 255, a: 1 };
    let opacity = 1;
    for (const ancestor of ancestors.reverse()) {
      const style = getComputedStyle(ancestor);
      opacity *= Number.parseFloat(style.opacity || '1');
      const color = parseColor(style.backgroundColor);
      color.a *= opacity;
      background = composite(color, background);
    }
    return { background, opacity };
  };

  const visible = (element: Element) => {
    if (element.closest('[aria-hidden="true"], .visually-hidden')) return false;
    const style = getComputedStyle(element);
    if (style.display === 'none' || style.visibility === 'hidden') return false;
    const bounds = element.getBoundingClientRect();
    return (
      bounds.width > 0 &&
      bounds.height > 0 &&
      bounds.right > 0 &&
      bounds.bottom > 0 &&
      bounds.left < window.innerWidth &&
      bounds.top < window.innerHeight
    );
  };

  const samples: RenderedTextContrast[] = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    const text = node.textContent?.replace(/\s+/g, ' ').trim();
    const element = node.parentElement;
    if (!text || !element || !visible(element)) continue;

    const style = getComputedStyle(element);
    const { background, opacity } = renderedBackground(element);
    const foreground = parseColor(style.color);
    foreground.a *= opacity;
    const renderedForeground = composite(foreground, background);
    const fontSize = Number.parseFloat(style.fontSize);
    const fontWeight = Number.parseInt(style.fontWeight, 10) || 400;
    const minimum = fontSize >= 24 || (fontSize >= 18.67 && fontWeight >= 700) ? 3 : 4.5;
    const inactive = element.closest(':disabled, [aria-disabled="true"]');
    const exemption = inactive ? 'inactive-control (WCAG 1.4.3)' : null;

    samples.push({
      text: text.slice(0, 72),
      element: `${element.tagName.toLowerCase()}${element.className ? `.${String(element.className).trim().replace(/\s+/g, '.')}` : ''}`,
      foreground: style.color,
      renderedForeground: cssColor(renderedForeground),
      background: cssColor(background),
      ratio: ratio(renderedForeground, background),
      minimum,
      exemption,
    });
  }

  for (const input of document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(
    'input, textarea',
  )) {
    if (!visible(input) || !input.placeholder) continue;
    const placeholder = getComputedStyle(input, '::placeholder');
    const { background, opacity } = renderedBackground(input);
    const foreground = parseColor(placeholder.color);
    foreground.a *= opacity * Number.parseFloat(placeholder.opacity || '1');
    const renderedForeground = composite(foreground, background);
    samples.push({
      text: `${input.placeholder} (placeholder)`,
      element: `${input.tagName.toLowerCase()}::placeholder`,
      foreground: placeholder.color,
      renderedForeground: cssColor(renderedForeground),
      background: cssColor(background),
      ratio: ratio(renderedForeground, background),
      minimum: 4.5,
      exemption: null,
    });
  }

  return samples;
}
