package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files

class PairedDesktopsTest {
    private val home = "a".repeat(ID_LEN)
    private val work = "b".repeat(ID_LEN)

    private fun withDir(test: (File) -> Unit) {
        val dir = Files.createTempDirectory("conduit-multipeer-test").toFile()
        try { test(dir) } finally { dir.deleteRecursively() }
    }

    @Test
    fun importsTheExistingTrustedDesktopWithoutChangingPhoneIdentity() = withDir { dir ->
        val oldIdentity = ByteArray(64) { it.toByte() }
        File(dir, "identity.bin").writeBytes(oldIdentity)
        File(dir, "peer.txt").writeText(home)
        File(dir, "peer-name.txt").writeText("Home PC")
        assertEquals(listOf(PairedDesktop(home, "Home PC")), PairedDesktops.list(dir))
        PairedDesktops.remember(dir, work)
        PairedDesktops.rename(dir, work, "Office PC")
        assertEquals(listOf(home, work), PairedDesktops.list(dir).map { it.id })
        assertEquals("Home PC", PairedDesktops.list(dir).first().name)
        assertEquals("Office PC", PairedDesktops.list(dir).last().name)
        assertTrue(File(dir, "identity.bin").readBytes().contentEquals(oldIdentity))
        assertEquals(work, Identity.peer(dir))
        assertEquals(work, PairedDesktops.selected(dir))
    }

    @Test
    fun independentSelectionDoesNotReplaceTheMainTransportPeer() = withDir { dir ->
        Identity.rememberPeer(dir, home)
        Identity.rememberPeerName(dir, "Home")
        Identity.rememberPeer(dir, work)
        Identity.rememberPeerName(dir, "Work")
        PairedDesktops.choose(dir, home)
        assertEquals(home, PairedDesktops.selected(dir))
        assertEquals(work, Identity.peer(dir))
        assertEquals("Work", Identity.peerName(dir))
        assertEquals(listOf("Home", "Work"), Identity.peers(dir).map { it.name })
    }

    @Test
    fun forgettingOneDesktopDoesNotRemoveOthers() = withDir { dir ->
        PairedDesktops.remember(dir, home)
        PairedDesktops.rename(dir, home, "Home")
        PairedDesktops.remember(dir, work)
        PairedDesktops.rename(dir, work, "Work")
        PairedDesktops.remove(dir, home)
        assertEquals(listOf(work), PairedDesktops.list(dir).map { it.id })
        PairedDesktops.remove(dir, work)
        assertEquals(emptyList<PairedDesktop>(), PairedDesktops.list(dir))
        assertFalse(File(dir, "peer.txt").exists())
    }

    @Test
    fun removingTheMainDevicePromotesAnExistingTrustedDevice() = withDir { dir ->
        PairedDesktops.remember(dir, home)
        PairedDesktops.rename(dir, home, "Home")
        PairedDesktops.remember(dir, work)
        PairedDesktops.remove(dir, work)
        assertEquals(home, Identity.peer(dir))
        assertEquals("Home", Identity.peerName(dir))
    }

    @Test
    fun malformedAndRepeatedIdsDoNotAddUntrustedPeers() = withDir { dir ->
        assertThrows(IllegalArgumentException::class.java) {
            PairedDesktops.remember(dir, "not-valid")
        }
        PairedDesktops.remember(dir, home)
        PairedDesktops.remember(dir, home)
        assertEquals(1, PairedDesktops.list(dir).size)
        File(dir, "paired-desktops.tsv").appendText("garbage\tnot-a-name\n")
        assertEquals(1, PairedDesktops.list(dir).size)
    }

    @Test
    fun directShareTargetsAreExactAndNeverFallBackToAnotherComputer() {
        val peers = listOf(PairedDesktop(home, "Home"), PairedDesktop(work, "Work"))
        assertEquals(home, PairedDesktops.targetFromShortcut(peers, "desktop-" + home.take(20)))
        assertEquals(work, PairedDesktops.targetFromShortcut(peers, "desktop-" + work.take(20)))
        assertEquals(null, PairedDesktops.targetFromShortcut(peers, "desktop-"))
        assertEquals(null, PairedDesktops.targetFromShortcut(peers, "desktop-" + "z".repeat(20)))
        assertEquals(null, PairedDesktops.targetFromShortcut(peers, "desktop"))
        assertEquals(null, PairedDesktops.targetFromShortcut(peers, null))
    }

    @Test
    fun peerRegistryIsBoundedAndDoesNotEvictTrustSilently() = withDir { dir ->
        for (i in 0 until PairedDesktops.MAX_PEERS) {
            PairedDesktops.remember(dir, ('A' + i).toString().repeat(ID_LEN))
        }
        assertThrows(IllegalArgumentException::class.java) {
            PairedDesktops.remember(dir, "z".repeat(ID_LEN))
        }
        assertEquals(PairedDesktops.MAX_PEERS, PairedDesktops.list(dir).size)
    }

    @Test
    fun pausedDesktopStaysPairedAndKeepsItsSwitchAcrossRenames() = withDir { dir ->
        PairedDesktops.remember(dir, home)
        PairedDesktops.rename(dir, home, "Home")
        PairedDesktops.setEnabled(dir, home, false)
        assertFalse(PairedDesktops.list(dir).single().enabled)
        PairedDesktops.rename(dir, home, "Home PC")
        PairedDesktops.choose(dir, home)
        assertEquals(listOf(PairedDesktop(home, "Home PC", enabled = false)), PairedDesktops.list(dir))
        // Re-pairing is an explicit "use this computer" and resumes it.
        PairedDesktops.remember(dir, home)
        assertTrue(PairedDesktops.list(dir).single().enabled)
    }

    @Test
    fun registryWritesThreeColumnsAndReadsTheLegacyTwoColumnForm() = withDir { dir ->
        PairedDesktops.remember(dir, home)
        PairedDesktops.rename(dir, home, "Home")
        PairedDesktops.setEnabled(dir, home, false)
        val row = File(dir, "paired-desktops.tsv").readText().trim()
        assertEquals(3, row.split('\t').size)
        assertEquals("0", row.split('\t')[2])
        // A v0.1.21 registry has no switch column at all, so every row means connected.
        File(dir, "paired-desktops.tsv").writeText(
            row.split('\t').take(2).joinToString("\t") + "\n",
        )
        assertTrue(PairedDesktops.list(dir).single().enabled)
    }

    @Test
    fun masterResumeReturnsEveryPausedComputerToTheRoster() = withDir { dir ->
        PairedDesktops.remember(dir, home)
        PairedDesktops.remember(dir, work)
        PairedDesktops.setEnabled(dir, home, false)
        PairedDesktops.setEnabled(dir, work, false)
        PairedDesktops.enableAll(dir)
        assertEquals(listOf(true, true), PairedDesktops.list(dir).map { it.enabled })
        // A peer that is already enabled is left alone instead of being rewritten.
        PairedDesktops.setEnabled(dir, work, true)
        assertEquals(listOf(true, true), PairedDesktops.list(dir).map { it.enabled })
    }
}
