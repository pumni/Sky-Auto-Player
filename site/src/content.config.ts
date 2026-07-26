import { defineCollection, z } from 'astro:content';
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

export const collections = { faq };
