import { isTauri } from '@tauri-apps/api/core';
import { Check, Coffee, Copy, ExternalLink, Heart, Landmark, WalletCards } from 'lucide-react';
import { useState, type MouseEvent } from 'react';
import { DONATION_CONFIG } from './donationConfig';

type Notice = { kind: 'success' | 'error'; text: string };
type CopyField = 'account' | 'memo';

export function DonateSettings() {
  const [notice, setNotice] = useState<Notice | null>(null);
  const [copiedField, setCopiedField] = useState<CopyField | null>(null);

  function openSupportLink(event: MouseEvent<HTMLAnchorElement>, url: string) {
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
          text: `Could not open the browser. Visit ${url} manually.`,
        });
      }
    })();
  }

  async function copyToClipboard(field: CopyField, value: string) {
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
        <span className="donate-heading-icon" aria-hidden="true">
          <Heart size={19} />
        </span>
        <div>
          <h3 id="donate-settings-title">Support Sky Auto Player</h3>
          <p className="settings-note">Support ongoing development if you enjoy Sky Auto Player.</p>
        </div>
      </div>

      <div className="donate-provider-grid">
        <div className="donate-provider-card">
          <div className="donate-provider-heading">
            <Coffee size={17} aria-hidden="true" />
            <div>
              <h4>Ko-fi</h4>
              <p>Support the project</p>
            </div>
          </div>
          <a
            className="button donate-external-link"
            href={DONATION_CONFIG.koFiUrl}
            aria-label="Open Ko-fi in browser"
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => openSupportLink(event, DONATION_CONFIG.koFiUrl)}
          >
            Open <ExternalLink size={14} aria-hidden="true" />
          </a>
        </div>
        <div className="donate-provider-card">
          <div className="donate-provider-heading">
            <WalletCards size={17} aria-hidden="true" />
            <div>
              <h4>PayPal</h4>
              <p>International support</p>
            </div>
          </div>
          <a
            className="button donate-external-link"
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

      <div className="donate-bank-card">
        <div className="donate-provider-heading">
          <Landmark size={17} aria-hidden="true" />
          <h4>MBBank · VietQR</h4>
        </div>
        <p className="settings-note">
          Scan with a banking app or transfer manually. Verify the recipient before confirming.
        </p>

        <div className="donate-bank-content">
          <figure className="donate-qr-card">
            <div className="donate-qr-frame">
              <img
                src={DONATION_CONFIG.vietQrImagePath}
                alt="MBBank VietQR for Nguyen Duc Hoa, account 0857288363"
                width={200}
                height={200}
              />
            </div>
            <figcaption>No fixed amount · VND</figcaption>
          </figure>

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
        Donations are optional. Sky Auto Player does not process or verify payments. You can also{' '}
        <a
          href={DONATION_CONFIG.githubUrl}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(event) => openSupportLink(event, DONATION_CONFIG.githubUrl)}
        >
          star on GitHub <ExternalLink size={12} aria-hidden="true" />
        </a>
        .
      </p>
    </section>
  );
}
