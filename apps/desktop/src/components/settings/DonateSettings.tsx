import { isTauri } from '@tauri-apps/api/core';
import { Check, Coffee, Copy, ExternalLink, Heart, Landmark } from 'lucide-react';
import { useState, type MouseEvent } from 'react';
import { DONATION_CONFIG } from './donationConfig';

type Notice = { kind: 'success' | 'error'; text: string };

export function DonateSettings() {
  const [notice, setNotice] = useState<Notice | null>(null);
  const [copiedField, setCopiedField] = useState<'account' | 'memo' | null>(null);

  function openSupportLink(event: MouseEvent<HTMLAnchorElement>, url: string) {
    // Regular links work in the browser test harness and on the website.
    // The packaged Tauri app must delegate links to the system browser.
    if (!isTauri()) return;
    event.preventDefault();
    setNotice(null);
    void (async () => {
      try {
        const { openUrl } = await import('@tauri-apps/plugin-opener');
        await openUrl(url);
      } catch {
        setNotice({
          kind: 'error',
          text: `Could not open the browser. You can visit ${url} manually.`,
        });
      }
    })();
  }

  async function copyToClipboard(field: 'account' | 'memo', value: string) {
    setNotice(null);
    setCopiedField(null);
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(field);
      setNotice({
        kind: 'success',
        text: `${field === 'account' ? 'Account number' : 'Transfer note'} copied.`,
      });
    } catch {
      setNotice({ kind: 'error', text: 'Could not copy. Select and copy the text manually.' });
    }
  }

  return (
    <section className="settings-section donate-settings" aria-labelledby="donate-settings-title">
      <div className="donate-heading">
        <div className="donate-heading-icon" aria-hidden="true">
          <Heart size={20} />
        </div>
        <div>
          <h3 id="donate-settings-title">Support Sky Auto Player</h3>
          <p className="settings-note">
            Sky Auto Player is free and open-source. If it makes your music experience better,
            consider supporting its continued development.
          </p>
        </div>
      </div>

      <div className="donate-provider-grid">
        <div className="donate-provider-card">
          <div className="donate-provider-heading">
            <Coffee size={17} aria-hidden="true" />
            <h4>Ko-fi</h4>
          </div>
          <p>Support the project through Ko-fi.</p>
          <a
            className="button donate-external-link"
            href={DONATION_CONFIG.koFiUrl}
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => openSupportLink(event, DONATION_CONFIG.koFiUrl)}
          >
            Open Ko-fi <ExternalLink size={14} aria-hidden="true" />
          </a>
        </div>

        <div className="donate-provider-card">
          <div className="donate-provider-heading">
            <Heart size={17} aria-hidden="true" />
            <h4>PayPal</h4>
          </div>
          <p>Support internationally through PayPal.Me.</p>
          <a
            className="button donate-external-link"
            href={DONATION_CONFIG.paypalUrl}
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => openSupportLink(event, DONATION_CONFIG.paypalUrl)}
          >
            Open PayPal <ExternalLink size={14} aria-hidden="true" />
          </a>
        </div>
      </div>

      <div className="donate-bank-card">
        <div className="donate-provider-heading">
          <Landmark size={17} aria-hidden="true" />
          <h4>Vietnam bank transfer · VietQR</h4>
        </div>
        <p className="settings-note">
          Scan with a Vietnamese banking app. Enter any amount, and verify the recipient before
          confirming the transfer.
        </p>
        <div className="donate-bank-content">
          <div className="donate-qr-frame">
            <img
              src={DONATION_CONFIG.vietQrImagePath}
              alt="VietQR for the listed Techcombank account, with no fixed transfer amount"
              width={172}
              height={172}
            />
          </div>
          <div className="donate-bank-information">
            <dl className="donate-bank-details">
              <div>
                <dt>Bank</dt>
                <dd>{DONATION_CONFIG.bankName}</dd>
              </div>
              <div>
                <dt>Account holder</dt>
                <dd>{DONATION_CONFIG.accountHolder}</dd>
              </div>
              <div>
                <dt>Account number</dt>
                <dd className="donate-bank-account">{DONATION_CONFIG.accountNumber}</dd>
              </div>
              <div>
                <dt>Transfer note</dt>
                <dd>{DONATION_CONFIG.transferMemo}</dd>
              </div>
            </dl>
            <div className="donate-copy-actions">
              <button
                className="button donate-copy-button"
                type="button"
                onClick={() => void copyToClipboard('account', DONATION_CONFIG.accountNumber)}
              >
                {copiedField === 'account' ? (
                  <Check size={14} aria-hidden="true" />
                ) : (
                  <Copy size={14} aria-hidden="true" />
                )}
                Copy account
              </button>
              <button
                className="button donate-copy-button"
                type="button"
                onClick={() => void copyToClipboard('memo', DONATION_CONFIG.transferMemo)}
              >
                {copiedField === 'memo' ? (
                  <Check size={14} aria-hidden="true" />
                ) : (
                  <Copy size={14} aria-hidden="true" />
                )}
                Copy note
              </button>
            </div>
          </div>
        </div>
      </div>

      {notice && (
        <p
          className={notice.kind === 'error' ? 'inline-error' : 'settings-note'}
          role={notice.kind === 'error' ? 'alert' : 'status'}
        >
          {notice.text}
        </p>
      )}
      <p className="settings-note donate-disclaimer">
        Contributions are entirely optional. Every core feature remains free. Payments take place
        outside Sky Auto Player; the app does not collect payment data or confirm transactions.
        You can also{' '}
        <a
          href={DONATION_CONFIG.githubUrl}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(event) => openSupportLink(event, DONATION_CONFIG.githubUrl)}
        >
          star the project on GitHub <ExternalLink size={12} aria-hidden="true" />
        </a>
        .
      </p>
    </section>
  );
}
