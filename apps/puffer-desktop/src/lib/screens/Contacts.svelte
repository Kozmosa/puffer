<script lang="ts">
  import { onMount } from "svelte";
  import {
    deleteContact,
    inferContacts,
    loadContacts,
    saveContact
  } from "../api/desktop";
  import type {
    ContactProposal,
    ContactsSnapshot,
    ConnectorContact,
    SavedContact
  } from "../types";

  let loading = $state(false);
  let inferring = $state(false);
  let saving = $state(false);
  let notice = $state<string | null>(null);
  let query = $state("");
  let snapshot = $state<ContactsSnapshot>({ contacts: [], candidates: [] });
  let proposals = $state<ContactProposal[]>([]);
  let editingId = $state<string | null>(null);
  let name = $state("");
  let description = $state("");
  let avatar = $state("");
  let contactIdsText = $state("");

  let visibleCandidates = $derived(
    snapshot.candidates.filter((candidate) => {
      const needle = query.trim().toLowerCase();
      if (!needle) return true;
      return candidate.id.toLowerCase().includes(needle)
        || (candidate.name ?? "").toLowerCase().includes(needle);
    })
  );

  onMount(() => {
    void refresh();
  });

  async function refresh() {
    loading = true;
    notice = null;
    try {
      snapshot = await loadContacts(80, query);
    } catch (err) {
      notice = `Could not load contacts: ${messageOf(err)}`;
    } finally {
      loading = false;
    }
  }

  async function infer() {
    inferring = true;
    notice = null;
    try {
      const result = await inferContacts(30);
      proposals = result.proposals;
      snapshot = { ...snapshot, candidates: result.candidates };
      notice = proposals.length === 0 ? "No contact proposals returned." : `Inferred ${proposals.length} contacts.`;
    } catch (err) {
      notice = `Could not infer contacts: ${messageOf(err)}`;
    } finally {
      inferring = false;
    }
  }

  function editContact(contact: SavedContact) {
    editingId = contact.id;
    name = contact.name;
    description = contact.description;
    avatar = contact.avatar ?? "";
    contactIdsText = contact.contact_ids.join("\n");
  }

  function editProposal(proposal: ContactProposal) {
    editingId = null;
    name = proposal.name;
    description = proposal.description;
    avatar = proposal.avatar ?? "";
    contactIdsText = proposal.contact_ids.join("\n");
  }

  function resetForm() {
    editingId = null;
    name = "";
    description = "";
    avatar = "";
    contactIdsText = "";
  }

  async function submitContact(event?: SubmitEvent) {
    event?.preventDefault();
    const ids = parsedContactIds();
    if (!name.trim() || ids.length === 0 || saving) return;
    saving = true;
    notice = null;
    try {
      snapshot = await saveContact({
        id: editingId ?? undefined,
        name,
        description,
        avatar: avatar.trim() || null,
        contact_ids: ids
      });
      notice = editingId ? `Updated ${name.trim()}.` : `Created ${name.trim()}.`;
      resetForm();
    } catch (err) {
      notice = `Could not save contact: ${messageOf(err)}`;
    } finally {
      saving = false;
    }
  }

  async function removeContact(contact: SavedContact) {
    if (saving) return;
    saving = true;
    notice = null;
    try {
      snapshot = await deleteContact(contact.id);
      if (editingId === contact.id) resetForm();
      notice = `Deleted ${contact.name}.`;
    } catch (err) {
      notice = `Could not delete contact: ${messageOf(err)}`;
    } finally {
      saving = false;
    }
  }

  function useCandidate(candidate: ConnectorContact) {
    const ids = new Set(parsedContactIds());
    ids.add(candidate.id);
    contactIdsText = Array.from(ids).join("\n");
    if (!name.trim()) name = candidate.name ?? candidate.id;
  }

  function parsedContactIds(): string[] {
    return contactIdsText
      .split(/[,\n]/)
      .map((value) => value.trim())
      .filter(Boolean);
  }

  function candidateSummary(candidate: ConnectorContact): string {
    const score = typeof candidate.score === "number" ? candidate.score.toFixed(2) : "0.00";
    const context = candidate.context?.[0]?.text?.trim();
    return context ? `${score} - ${context}` : score;
  }

  function messageOf(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
  }
</script>

<div class="pf-contacts">
  <header class="pf-contacts-head">
    <div>
      <h1>Contacts</h1>
      <p>Grouped connector identities for monitor task subscriptions.</p>
    </div>
    <div class="pf-contacts-actions">
      <label class="pf-contacts-search">
        <span>Search</span>
        <input bind:value={query} oninput={() => void refresh()} placeholder="telegram@alice, google@..." />
      </label>
      <button type="button" class="pf-btn" onclick={() => void refresh()} disabled={loading}>
        {loading ? "Loading" : "Refresh"}
      </button>
      <button type="button" class="pf-btn" data-variant="secondary" onclick={() => void infer()} disabled={inferring}>
        {inferring ? "Inferring" : "Infer"}
      </button>
    </div>
  </header>

  {#if notice}
    <p class="pf-contacts-notice">{notice}</p>
  {/if}

  <div class="pf-contacts-grid">
    <section class="pf-contacts-panel">
      <div class="pf-panel-head">
        <strong>Saved</strong>
        <span>{snapshot.contacts.length}</span>
      </div>
      {#if snapshot.contacts.length === 0}
        <p class="pf-empty">No saved contacts yet.</p>
      {:else}
        <div class="pf-contact-list">
          {#each snapshot.contacts as contact (contact.id)}
            <article class="pf-contact-card">
              <div>
                <strong>{contact.name}</strong>
                <p>{contact.description || "No description."}</p>
                <div class="pf-contact-ids">
                  {#each contact.contact_ids as id}
                    <code>{id}</code>
                  {/each}
                </div>
              </div>
              <div class="pf-card-actions">
                <button type="button" onclick={() => editContact(contact)}>Edit</button>
                <button type="button" onclick={() => void removeContact(contact)} disabled={saving}>Delete</button>
              </div>
            </article>
          {/each}
        </div>
      {/if}
    </section>

    <section class="pf-contacts-panel">
      <div class="pf-panel-head">
        <strong>{editingId ? "Edit Contact" : "Create Contact"}</strong>
        <button type="button" onclick={resetForm}>Clear</button>
      </div>
      <form class="pf-contact-form" onsubmit={(event) => void submitContact(event)}>
        <label>
          <span>Name</span>
          <input bind:value={name} required />
        </label>
        <label>
          <span>Description</span>
          <textarea bind:value={description} rows="5"></textarea>
        </label>
        <label>
          <span>Avatar</span>
          <input bind:value={avatar} placeholder="optional URL" />
        </label>
        <label>
          <span>Contact IDs</span>
          <textarea bind:value={contactIdsText} rows="6" placeholder="telegram@alice&#10;google@alice@example.com"></textarea>
        </label>
        <button type="submit" class="pf-btn" disabled={saving || !name.trim() || parsedContactIds().length === 0}>
          {saving ? "Saving" : editingId ? "Update" : "Create"}
        </button>
      </form>
    </section>

    <section class="pf-contacts-panel">
      <div class="pf-panel-head">
        <strong>Candidates</strong>
        <span>{visibleCandidates.length}</span>
      </div>
      <div class="pf-candidate-list">
        {#each visibleCandidates as candidate (candidate.id)}
          <button type="button" class="pf-candidate" onclick={() => useCandidate(candidate)}>
            <strong>{candidate.name ?? candidate.id}</strong>
            <code>{candidate.id}</code>
            <span>{candidateSummary(candidate)}</span>
          </button>
        {/each}
        {#if visibleCandidates.length === 0}
          <p class="pf-empty">No connector candidates.</p>
        {/if}
      </div>
    </section>

    <section class="pf-contacts-panel">
      <div class="pf-panel-head">
        <strong>Inferred</strong>
        <span>{proposals.length}</span>
      </div>
      {#if proposals.length === 0}
        <p class="pf-empty">Run inference to propose grouped contacts.</p>
      {:else}
        <div class="pf-contact-list">
          {#each proposals as proposal, index (`${proposal.name}-${index}`)}
            <article class="pf-contact-card">
              <div>
                <strong>{proposal.name}</strong>
                <p>{proposal.description}</p>
                <div class="pf-contact-ids">
                  {#each proposal.contact_ids as id}
                    <code>{id}</code>
                  {/each}
                </div>
              </div>
              <div class="pf-card-actions">
                <button type="button" onclick={() => editProposal(proposal)}>Use</button>
              </div>
            </article>
          {/each}
        </div>
      {/if}
    </section>
  </div>
</div>

<style>
  .pf-contacts {
    height: 100%;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow: auto;
  }

  .pf-contacts-head,
  .pf-contacts-actions,
  .pf-panel-head,
  .pf-card-actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .pf-contacts-head {
    justify-content: space-between;
  }

  .pf-contacts-head h1 {
    margin: 0;
    font-size: 20px;
  }

  .pf-contacts-head p,
  .pf-empty,
  .pf-contact-card p,
  .pf-contacts-notice {
    margin: 0;
    color: var(--muted-foreground);
    font-size: 12px;
    line-height: 1.4;
  }

  .pf-contacts-search {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--muted-foreground);
  }

  .pf-contacts input,
  .pf-contacts textarea {
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--background);
    color: var(--foreground);
    padding: 7px 8px;
    font: inherit;
    font-size: 12px;
  }

  .pf-contacts-search input {
    width: min(360px, 32vw);
  }

  .pf-btn,
  .pf-card-actions button,
  .pf-panel-head button {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--foreground);
    color: var(--background);
    padding: 7px 11px;
    font-size: 12px;
    cursor: pointer;
  }

  .pf-btn[data-variant="secondary"],
  .pf-card-actions button,
  .pf-panel-head button {
    background: var(--background);
    color: var(--foreground);
  }

  button:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .pf-contacts-grid {
    display: grid;
    grid-template-columns: minmax(260px, 1fr) minmax(280px, 0.9fr);
    gap: 12px;
    align-items: start;
  }

  .pf-contacts-panel {
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--background);
    min-height: 160px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .pf-panel-head {
    justify-content: space-between;
    min-height: 28px;
  }

  .pf-panel-head span {
    color: var(--muted-foreground);
    font-size: 12px;
  }

  .pf-contact-list,
  .pf-candidate-list,
  .pf-contact-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .pf-contact-form label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--muted-foreground);
  }

  .pf-contact-card,
  .pf-candidate {
    border: 1px solid color-mix(in oklab, var(--border) 78%, transparent);
    border-radius: 8px;
    padding: 10px;
    background: color-mix(in oklab, var(--muted) 18%, var(--background));
  }

  .pf-contact-card {
    display: flex;
    justify-content: space-between;
    gap: 12px;
  }

  .pf-candidate {
    width: 100%;
    text-align: left;
    display: grid;
    grid-template-columns: minmax(120px, 0.7fr) minmax(130px, 0.8fr) minmax(140px, 1fr);
    gap: 8px;
    color: var(--foreground);
    cursor: pointer;
  }

  .pf-candidate span {
    color: var(--muted-foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pf-contact-ids {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    margin-top: 8px;
  }

  .pf-contacts code {
    font-size: 11px;
    color: var(--foreground);
    background: color-mix(in oklab, var(--muted) 30%, transparent);
    border-radius: 5px;
    padding: 2px 5px;
  }

  @media (max-width: 980px) {
    .pf-contacts-head,
    .pf-contacts-actions,
    .pf-contact-card {
      align-items: stretch;
      flex-direction: column;
    }

    .pf-contacts-grid {
      grid-template-columns: 1fr;
    }

    .pf-candidate {
      grid-template-columns: 1fr;
    }

    .pf-contacts-search input {
      width: 100%;
    }
  }
</style>
