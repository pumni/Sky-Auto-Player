import { Coffee, ExternalLink, HandHeart, Landmark, WalletCards } from 'lucide-react';
import { type MouseEvent, useState } from 'react';
import { openExternalUrl } from '../../platform/externalLinks';
import { isTauriRuntime } from '../../platform/runtime';
import { SettingsCategoryHeader, SettingsSection } from './SettingsLayout';
import { DONATION_CONFIG } from './donationConfig';

export function DonateSettings() {
  const [errorNotice, setErrorNotice] = useState<string | null>(null);

  function openSupportLink(event: MouseEvent<HTMLAnchorElement>, url: string) {
    if (!isTauriRuntime()) return;
    event.preventDefault();
    setErrorNotice(null);
    void (async () => {
      try {
        await openExternalUrl(url);
      } catch {
        setErrorNotice(`Could not open the browser. Visit ${url} manually.`);
      }
    })();
  }

  return (
    <section
      data-settings-category="donate"
      data-slot="settings-category-content"
      className="grid content-start gap-4"
      aria-labelledby="donate-settings-title"
    >
      <SettingsCategoryHeader
        id="donate-settings-title"
        title="Support Sky Auto Player"
        description="Support ongoing development if you enjoy Sky Auto Player."
        icon={<HandHeart size={18} className="shrink-0 text-text-secondary" aria-hidden="true" />}
      />

      <SettingsSection title="Online support" divider>
        <div className="grid grid-cols-1 divide-y divide-border">
          <div
            data-slot="donate-provider-row"
            className="flex min-w-0 items-center justify-between gap-3 pb-2.5"
          >
            <div data-slot="donate-provider-heading" className="flex min-w-0 items-center gap-2.5">
              <Coffee className="shrink-0 text-text-secondary" size={16} aria-hidden="true" />
              <div className="min-w-0">
                <h5 className="m-0 text-[13px] font-medium leading-5 text-foreground">Ko-fi</h5>
                <p className="m-0 text-xs leading-[18px] text-text-secondary">
                  Support the project
                </p>
              </div>
            </div>
            <a
              data-slot="donate-external-link"
              className="inline-flex h-8 flex-none items-center gap-1.5 rounded-control border border-border bg-card px-2.5 text-xs font-medium text-text-secondary transition-colors duration-fast hover:bg-control-hover hover:text-foreground motion-reduce:transition-none"
              href={DONATION_CONFIG.koFiUrl}
              aria-label="Open Ko-fi in browser"
              target="_blank"
              rel="noopener noreferrer"
              onClick={(event) => openSupportLink(event, DONATION_CONFIG.koFiUrl)}
            >
              Open <ExternalLink size={14} aria-hidden="true" />
            </a>
          </div>

          <div
            data-slot="donate-provider-row"
            className="flex min-w-0 items-center justify-between gap-3 pt-2.5"
          >
            <div data-slot="donate-provider-heading" className="flex min-w-0 items-center gap-2.5">
              <WalletCards className="shrink-0 text-text-secondary" size={16} aria-hidden="true" />
              <div className="min-w-0">
                <h5 className="m-0 text-[13px] font-medium leading-5 text-foreground">PayPal</h5>
                <p className="m-0 text-xs leading-[18px] text-text-secondary">
                  International support
                </p>
              </div>
            </div>
            <a
              data-slot="donate-external-link"
              className="inline-flex h-8 flex-none items-center gap-1.5 rounded-control border border-border bg-card px-2.5 text-xs font-medium text-text-secondary transition-colors duration-fast hover:bg-control-hover hover:text-foreground motion-reduce:transition-none"
              href={DONATION_CONFIG.paypalUrl}
              aria-label="Open PayPal in browser"
              target="_blank"
              rel="noopener noreferrer"
              onClick={(event) => openSupportLink(event, DONATION_CONFIG.paypalUrl)}
            >
              Open <ExternalLink size={14} aria-hidden="true" />
            </a>
          </div>
        </div>
      </SettingsSection>

      <div
        data-slot="donate-bank-card"
        className="grid gap-3 rounded-pane border border-border bg-card p-3.5"
      >
        <div className="flex items-center gap-2">
          <Landmark size={16} className="shrink-0 text-text-secondary" aria-hidden="true" />
          <h4 className="m-0 text-sm font-semibold leading-5 text-foreground">
            Bank transfer · VietQR
          </h4>
        </div>
        <div className="grid min-w-0 grid-cols-1 items-start gap-4 @min-[440px]/settings-content:grid-cols-[160px_minmax(0,1fr)]">
          <div
            data-slot="donate-qr-frame"
            className="grid size-40 place-items-center rounded-control border border-border bg-white p-2"
          >
            <img
              src={DONATION_CONFIG.vietQrImagePath}
              alt="VietQR bank transfer code"
              className="size-full object-contain"
              width={200}
              height={200}
            />
          </div>

          <div data-slot="donate-bank-information" className="min-w-0">
            <p
              data-slot="donate-bank-guidance"
              className="m-0 text-xs leading-[18px] text-text-secondary"
            >
              Scan with your phone’s banking app. Verify the recipient before confirming.
            </p>
            <dl data-slot="donate-bank-details" className="mb-0 mt-2 grid gap-0">
              <div className="grid grid-cols-[110px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-1 last:border-b-0">
                <dt className="text-xs text-text-tertiary">Bank</dt>
                <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                  {DONATION_CONFIG.bankName}
                </dd>
              </div>
              <div className="grid grid-cols-[110px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-1 last:border-b-0">
                <dt className="text-xs text-text-tertiary">Account holder</dt>
                <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                  {DONATION_CONFIG.accountHolder}
                </dd>
              </div>
              <div
                data-slot="donate-bank-reference-row"
                className="grid grid-cols-[110px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-1 last:border-b-0"
              >
                <dt className="text-xs text-text-tertiary">Account number</dt>
                <dd className="m-0 min-w-0 break-words font-mono text-xs font-semibold tabular-nums text-foreground">
                  {DONATION_CONFIG.accountNumber}
                </dd>
              </div>
              <div
                data-slot="donate-bank-reference-row"
                className="grid grid-cols-[110px_minmax(0,1fr)] items-baseline gap-3 py-1"
              >
                <dt className="text-xs text-text-tertiary">Transfer note</dt>
                <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                  {DONATION_CONFIG.transferMemo}
                </dd>
              </div>
            </dl>
          </div>
        </div>
      </div>

      {errorNotice && (
        <p className="m-0 text-xs text-danger" role="alert">
          {errorNotice}
        </p>
      )}

      <p className="m-0 text-xs leading-[18px] text-text-tertiary [&_a]:inline-flex [&_a]:items-center [&_a]:gap-1 [&_a]:font-medium [&_a]:text-accent-ink [&_a]:underline-offset-2 hover:[&_a]:underline focus-within:[&_a]:outline-2 focus-within:[&_a]:outline-ring">
        Donations are optional. Sky Auto Player does not process or verify payments. You can also{' '}
        <a
          href={DONATION_CONFIG.githubUrl}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(event) => openSupportLink(event, DONATION_CONFIG.githubUrl)}
        >
          <span>star on GitHub</span>
          <ExternalLink size={12} aria-hidden="true" />
        </a>
        .
      </p>
    </section>
  );
}
