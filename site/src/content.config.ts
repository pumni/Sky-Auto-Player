import { defineCollection } from 'astro:content';
import { z } from 'astro/zod';
import { glob } from 'astro/loaders';

const faq = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/faq' }),
  schema: z.object({
    key: z.string().min(1),
    locale: z.enum(['en', 'vi']),
    order: z.number().int(),
    category: z.string().min(1),
    question: z.string().min(1),
  }),
});

const guides = defineCollection({
  loader: glob({
    pattern: '**/*.md',
    base: './src/content/guides',
    generateId: ({ entry }) => entry.replace(/\.md$/, ''),
  }),
  schema: z.object({
    /** Shared key across EN/VI pairs. Example: 'timing-engine' */
    key: z.string().min(1),
    locale: z.enum(['en', 'vi']),
    /** URL slug — must match directory name under pages/guides/ */
    slug: z.string().min(1),
    title: z.string().min(1),
    description: z.string().min(1),
    /** One-paragraph human-readable summary for hub listing and JSON-LD description. */
    summary: z.string().min(1),
    category: z.enum(['getting-started', 'playback-timing', 'technical-safety', 'support']),
    /** Display order within category on the hub page. Lower = earlier. */
    order: z.number().int(),
    /** ISO 8601 date: YYYY-MM-DD */
    published: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
    /** ISO 8601 date: YYYY-MM-DD. Only update when content actually changes. */
    updated: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
    /** Application version this guide was last reviewed against. */
    lastReviewedVersion: z.string().min(1),
    /** Draft guides are excluded from routes and sitemap. */
    draft: z.boolean().default(false),
    /** Optional page-specific OG image path (relative to public/). */
    image: z.string().optional(),
    /** Alt text for the page-specific OG image. */
    imageAlt: z.string().optional(),
    /**
     * At least one evidence source required for factual/technical guides.
     * Links should point to GitHub repository source, not the Pages mirror.
     */
    evidence: z
      .array(
        z.object({
          category: z.enum([
            'architecture',
            'implementation',
            'test',
            'release',
            'security',
            'distribution',
          ]),
          label: z.string().min(1),
          url: z.string().url(),
        }),
      )
      .min(1),
    /** When true, renders the ArchitectureDiagram component below the prose. */
    showDiagram: z.boolean().optional().default(false),
  }),
});

export const collections = { faq, guides };
