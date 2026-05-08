/**
 * AI features repository — face/object/pet clusters and global AI controls.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class AiRepository @Inject constructor(private val api: ApiService) {

    suspend fun status(): AiStatusResponse = api.getAiStatus()

    suspend fun toggle(enabled: Boolean): AiToggleResponse =
        api.toggleAi(AiToggleRequest(enabled))

    suspend fun reprocess(scope: String? = null): AiReprocessResponse =
        api.reprocessAi(AiReprocessRequest(scope))

    // Face clusters
    suspend fun listFaceClusters(): List<FaceCluster> =
        api.listFaceClusters().clusters

    suspend fun mergeFaceClusters(source: String, target: String) {
        api.mergeFaceClusters(FaceClusterMergeRequest(source, target))
    }

    suspend fun splitFaceCluster(clusterId: String, faceIds: List<String>) {
        api.splitFaceCluster(FaceClusterSplitRequest(clusterId, faceIds))
    }

    suspend fun listFaceClusterPhotos(clusterId: String): List<FaceClusterPhotoEntry> =
        api.listFaceClusterPhotos(clusterId).photos

    suspend fun renameFaceCluster(clusterId: String, name: String) {
        api.renameFaceCluster(clusterId, FaceClusterRenameRequest(name))
    }

    // Object classes
    suspend fun listObjectClasses(): List<ObjectClass> =
        api.listObjectClasses().classes

    suspend fun listObjectClassPhotos(className: String): List<ObjectClassPhotoEntry> =
        api.listObjectClassPhotos(className).photos

    // Pet clusters
    suspend fun listPetClusters(): List<PetCluster> =
        api.listPetClusters().clusters

    suspend fun mergePetClusters(source: String, target: String) {
        api.mergePetClusters(PetClusterMergeRequest(source, target))
    }

    suspend fun listPetClusterPhotos(clusterId: String): List<PetClusterPhotoEntry> =
        api.listPetClusterPhotos(clusterId).photos

    suspend fun renamePetCluster(clusterId: String, name: String) {
        api.renamePetCluster(clusterId, PetClusterRenameRequest(name))
    }
}
