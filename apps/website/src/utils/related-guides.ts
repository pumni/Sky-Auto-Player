import type { CollectionEntry } from 'astro:content';

export interface RelatedGuide {
  title: string;
  slug: string;
}

export function resolveRelatedGuides(
  guide: CollectionEntry<'guides'>,
  candidates: CollectionEntry<'guides'>[],
): RelatedGuide[] {
  const guideByKey = new Map(candidates.map((candidate) => [candidate.data.key, candidate]));
  const seenKeys = new Set<string>();

  return guide.data.related.map((relatedKey) => {
    if (relatedKey === guide.data.key) {
      throw new Error(
        `[related-guides] Guide "${guide.data.slug}" (${guide.data.locale}) self-references its own key: "${relatedKey}"`,
      );
    }
    if (seenKeys.has(relatedKey)) {
      throw new Error(
        `[related-guides] Guide "${guide.data.slug}" (${guide.data.locale}) has duplicate related key: "${relatedKey}"`,
      );
    }
    seenKeys.add(relatedKey);

    const target = guideByKey.get(relatedKey);
    if (!target) {
      throw new Error(
        `[related-guides] Guide "${guide.data.slug}" (${guide.data.locale}) references missing or draft target key: "${relatedKey}"`,
      );
    }
    return {
      title: target.data.title,
      slug: target.data.slug,
    };
  });
}
