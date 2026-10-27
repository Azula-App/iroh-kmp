package app.azula.iroh

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.test.Test
import kotlin.test.assertEquals

class MacosEchoTest {
    private fun local() = EndpointOptions(
        alpns = listOf(ALPN), relayMode = RelayModeOption.Disabled, addressLookup = false, warmUpOnline = false,
        bindAddr = "127.0.0.1:0",
    )

    @Test
    fun echoOnLoopback() = runBlocking {
        withTimeout(20_000) {
            val server = IrohEndpoint.bindWith(local())
            val client = IrohEndpoint.bindWith(local())
            echo(server, client) { client.connectAddr(server.addr(), ALPN) }
        }
    }

    @Test
    fun echoThroughN0ByTicket() = runBlocking {
        withTimeout(40_000) {
            val server = IrohEndpoint.bind(listOf(ALPN), null)
            val client = IrohEndpoint.bind(listOf(ALPN), null)
            val ticket = server.myTicket()
            echo(server, client) { client.connectConn(ticket, ALPN) }
        }
    }

    private suspend fun CoroutineScope.echo(server: IrohEndpoint, client: IrohEndpoint, dial: suspend () -> IrohConnection) {
        val incoming = async(Dispatchers.Default) { server.acceptNext()!! }
        val stream = dial().openBi()
        stream.sendBytes("ping".encodeToByteArray())
        val accepted = incoming.await()
        assertEquals(client.id(), accepted.remoteId)
        assertEquals("ping", accepted.stream.readExact(4u).decodeToString())
        accepted.stream.sendBytes("pong".encodeToByteArray())
        assertEquals("pong", stream.readExact(4u).decodeToString())
        client.shutdown()
        server.shutdown()
    }

    private companion object {
        const val ALPN = "iroh-kmp/echo"
    }
}
