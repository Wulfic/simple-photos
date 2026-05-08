/**
 * Setup wizard repository — discover, status/init/finalize, pair with
 * primary, verify backup.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class SetupRepository @Inject constructor(private val api: ApiService) {

    suspend fun discoverInfo(): DiscoverInfoResponse = api.discoverInfo()

    suspend fun status(): SetupStatusResponse = api.getSetupStatus()

    suspend fun init(username: String, password: String): SetupInitResponse =
        api.setupInit(SetupInitRequest(username, password))

    suspend fun finalize(installType: String? = null, serverRole: String? = null): SetupFinalizeResponse =
        api.setupFinalize(SetupFinalizeRequest(installType, serverRole))

    suspend fun discover(): List<SetupDiscoverServer> = api.setupDiscover().servers

    suspend fun pair(
        mainServerUrl: String,
        username: String,
        password: String,
    ): SetupPairResponse =
        api.setupPair(SetupPairRequest(mainServerUrl, username, password))

    suspend fun verifyBackup(
        address: String,
        username: String,
        password: String,
    ): VerifyBackupResponse =
        api.setupVerifyBackup(VerifyBackupRequest(address, username, password))
}
